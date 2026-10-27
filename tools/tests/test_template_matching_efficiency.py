"""Exercise provenance, immutable profiles, failed rows, and numerical acceptance without Cargo."""

import contextlib
import copy
import importlib.util
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "template_matching_efficiency", Path(__file__).resolve().parents[1] / "template-matching-efficiency.py"
)
collector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(collector)
TARGET = "aarch64-apple-darwin"
NATIVE_TARGET = collector.TARGETS[1] if os.name == "nt" else TARGET
RUNTIME = {"method": "dyld-loaded-image-paths",
           "images": [{"path": "/runtime/libopencv_core.dylib", "sha256": "e" * 64}]}


def format_observation():
    declaration = collector.direct_format_contract()
    cases = [{key: case[key] for key in ("id", "source_format", "selected_format")}
             | {"correct": True, "work": {key: case[f"expected_{key}"] for key in
                ("mapping_copies", "mapping_copied_bytes", "bgr_conversions", "bgr_converted_bytes")}}
             for case in declaration["cases"]]
    for case, expected in zip(cases, declaration["cases"]):
        source_stride = 1920 * 4 + expected["row_padding_bytes"]
        case["source_stride"] = source_stride
        case["mapped_stride"] = 320 * 4 if expected["nonzero_roi"] else 1920 * 4 if expected["forced_bgra_public_mapping"] else source_stride
        case["mapping_shared"] = not expected["nonzero_roi"] and not expected["forced_bgra_public_mapping"]
    lifetimes = []
    for name, order, held, evicted, active in (
        ("persistent-hit", collector.PERSISTENT_ORDER, (0, 172800, 172800, 172800, 172800, 172800, 0),
         (0,) * 7, (0,) * 7),
        ("active-evicted", collector.ACTIVE_ORDER, (6220800, 12441600, 12441600, 12441600, 12441600, 0),
         (0, 6220800, 0, 0, 0, 0), (1, 1, 0, 0, 0, 0)),
    ):
        checkpoints = [{"checkpoint": point, "cache_held_bgr_bytes": cache_bytes,
                        "active_evicted_bgr_bytes": evicted_bytes, "active_backend_finds": finds,
                        "mapping_owner_live": False, "source_frame_alive": False,
                        "result_owner_live": False}
                       for point, cache_bytes, evicted_bytes, finds in zip(order, held, evicted, active)]
        lifetimes.append({"id": name, "checkpoints": checkpoints})
        owners = (
            ((True, True, True, False, False, False, False), (True, True, True, True, False, False, False), (False, True, True, True, True, False, False))
            if name == "persistent-hit" else
            ((True, True, False, False, False, False), (True, True, True, False, False, False), (False, False, True, True, False, False))
        )
        for point, mapped, source, result in zip(checkpoints, *owners):
            point.update(mapping_owner_live=mapped, source_frame_alive=source, result_owner_live=result)
    transitions = [{key: expected[key] for key in ("id", "selected_format", "region")} |
                   {"correct": True, "work": {key: expected[key] for key in
                    ("mapping_copies", "mapping_copied_bytes", "bgr_conversions", "bgr_converted_bytes")}}
                   for expected in declaration["transition_cases"]]
    return {"correct": True, "cases": cases, "transitions": transitions, "lifetimes": lifetimes,
            "work": {key: value for key, value in collector.DIRECT_FORMAT_WORK.items()
                     if key != "backend_find_calls"},
            "backend_find_calls": 18, "backend_find_completions": 18}


def manifest():
    rows = []
    for definition in collector.DEFINITIONS.values():
        rows.append({**definition, "cache_state": "fresh-frame-and-backend",
                     "timing_boundary": "all public searches; setup excluded",
                     "width": 1920, "height": 1080, "result_limit": 16,
                     "suppression": "KeepAll", "warmups": 20 if definition["statistical"] else 1,
                     "sample_count": 200 if definition["statistical"] else 3})
        if definition["id"] == collector.DIRECT_FORMAT_WORKLOAD:
            rows[-1]["format_contract"] = collector.direct_format_contract()
    return {"format_version": 1, "kind": "manifest", "target": TARGET,
            "loaded_native_runtime": copy.deepcopy(RUNTIME),
            "opencv_version": "4.12.0", "fixture_sha256": "f" * 64, "workloads": rows}


def payload(workload, *, candidate=False, smoke=False):
    row = copy.deepcopy(next(row for row in manifest()["workloads"] if row["id"] == workload))
    if smoke:
        row.update(warmups=1, sample_count=3)
    count, warmups = row["sample_count"], row["warmups"]
    elapsed, cpu = (500_000, 250_000) if candidate else (1_000_000, 500_000)
    row["samples"] = [{"elapsed_ns": elapsed, "cpu_ns": cpu, "correct": True} for _ in range(count)]
    row["warmup_samples"] = [{"elapsed_ns": elapsed, "cpu_ns": cpu, "correct": True} for _ in range(warmups)]
    row["oracle_failures"] = 0
    iterations = warmups + count
    row["metrics"] = {
        "iterations": iterations, "cpu_ns": count * cpu, "cpu_reason": None, "native_resource_reason": None,
        "peak_rss_bytes": 16 * collector.MIB, "rss_before_bytes": 10 * collector.MIB,
        "rss_after_bytes": 12 * collector.MIB, "rss_after_drop_bytes": 10 * collector.MIB,
        "rss_steady_before_bytes": 12 * collector.MIB, "rss_steady_after_bytes": 12 * collector.MIB,
        "peak_heap_bytes": 8 * collector.MIB, "heap_before_bytes": 4 * collector.MIB,
        "heap_after_bytes": 6 * collector.MIB, "heap_after_drop_bytes": 4 * collector.MIB,
        "heap_steady_before_bytes": 6 * collector.MIB, "heap_steady_after_bytes": 6 * collector.MIB,
        "peak_rss_scope": "process-high-water", "peak_heap_scope": "row-including-setup-and-warmup",
        "allocations": iterations * 10,
        "score_copied_bytes": 0 if candidate else 4096 * iterations,
        "score_offsets_scanned": (1000 if candidate else 100_000) * iterations,
        "bgr_conversions": (1 if candidate else 8) * iterations,
        "bgr_converted_bytes": (1 if candidate else 8) * iterations * 1024,
        "bgr_retained_bytes": 1024 if candidate else 0,
        "bgr_retained_peak_bytes": 1024 if candidate else 0,
        "bgr_retained_after_drop_bytes": 0,
        "bgr_evicted_bytes": 0, "bgr_evicted_peak_bytes": 1024 if candidate else 0,
        "bgr_evicted_after_drop_bytes": 0,
        "mapping_copies": (1 if candidate else 9) * iterations,
        "mapping_copied_bytes": (1 if candidate else 9) * iterations * 1024,
    }
    if workload == collector.DIRECT_FORMAT_WORKLOAD:
        row["format_observations"] = [format_observation() for _ in range(iterations)]
        row["metrics"].update({key: value * iterations for key, value in collector.DIRECT_FORMAT_WORK.items()})
        row["metrics"].update(bgr_retained_bytes=0, bgr_retained_peak_bytes=12441600,
                              bgr_evicted_peak_bytes=6220800, final_owner_correct=True)
    return {"format_version": 1, "kind": "observations", "target": TARGET,
            "loaded_native_runtime": copy.deepcopy(RUNTIME),
            "opencv_version": "4.12.0", "fixture_sha256": "f" * 64,
            "warmups": 1 if smoke else 20, "sample_count": 3 if smoke else 200,
            "timer_resolution_ns": 10, "cpu_timer_resolution_ns": 1000,
            "timer_resolution_method": "minimum-observed-positive-Instant-delta",
            "cpu_resolution_method": "minimum-observed-positive-process-cpu-delta", "workloads": [row]}


def identity(root, role):
    root = PurePosixPath("/binary-root") / root.name
    return {"path": str(root / role), "sha256": ("a" if role == "baseline" else "b") * 64,
            "metadata_path": str(root / f"{role}.json"), "metadata_sha256": "d" * 64,
            "build": {"target": TARGET, "source_revision": ("a" if role == "baseline" else "b") * 40,
                      "source_tree": "c" * 40, "cargo_lock_sha256": "d" * 64,
                      "binary_sha256": ("a" if role == "baseline" else "b") * 64,
                      "rustc": "rustc 1.97.1", "build_argv": ["cargo", "bench", "--no-run", "--locked"],
                      "opencv_version": "4.12.0", "dependencies": copy.deepcopy(RUNTIME["images"])}}


def plan(root, mode):
    roles = ("baseline", "candidate") if mode == "comparison" else ("baseline",)
    rows = list(collector.DEFINITIONS)
    execution = {"execution_limits": collector.SMOKE_LIMITS} if mode == "smoke" else {}
    return {"format_version": 1, "kind": "campaign", "mode": mode, "target": TARGET,
            "host": {"target": TARGET, "node": "controlled-host"}, "collector": collector.collector_identity(),
            "binaries": {role: identity(root, role) for role in roles}, "environment": {},
            "policy": copy.deepcopy(collector.POLICY), "workloads": rows,
            "schedule": collector.schedule(mode, rows), "exclusive_host": True, "cwd": str(root), **execution}


def parsed_evidence(root, mode):
    specification = plan(root, mode)
    frozen = collector.manifest_identity(manifest(), TARGET, "4.12.0")
    cohorts = {}
    for role in specification["binaries"]:
        cohorts[role] = {
            workload: [{**collector.observations(payload(workload, candidate=role == "candidate"), frozen, workload, False),
                        "process": process} for process in range(5)]
            for workload in collector.DEFINITIONS
        }
    return {"plan": specification, "plan_sha256": "1" * 64, "attempts": [], "manifest": frozen,
            "observations": cohorts, "issues": []}


def persist_campaign(root, mode, profile=None, smoke=False):
    root.mkdir()
    (root / "attempts").mkdir()
    specification = plan(root.parent, mode)
    if smoke:
        specification["workloads"] = ["single-small96x64"]
        specification["schedule"] = collector.schedule("smoke", specification["workloads"])
    if profile is not None:
        collector.save(root / "profile.json", profile)
        specification["profile_sha256"] = collector.sha256(root / "profile.json")
    collector.save(root / "plan.json", specification)
    for task in specification["schedule"]:
        binary = specification["binaries"][task["role"]]
        value = manifest() if task["workload"] is None else payload(
            task["workload"], candidate=task["role"] == "candidate", smoke=smoke)
        record = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
                  "launch_error": None, "duration_seconds": 1.0, "stdout": json.dumps(value), "stderr": ""}
        collector.save(root / "attempts" / f"{task['id']}.json", {
            "task": task, "argv": collector.command(binary, task, mode),
            "plan_sha256": collector.sha256(root / "plan.json"), "identity_before": binary,
            "identity_after": binary, "identity_error": None, "record": record, "timeout_seconds": 90 if smoke else 300})
    return specification


class ParserTests(unittest.TestCase):
    def setUp(self):
        self.workload = "selection-k16-dense1080p-keepall"
        self.manifest = collector.manifest_identity(manifest(), TARGET, "4.12.0")

    def parse(self, value, smoke=False):
        return collector.observations(value, self.manifest, self.workload, smoke)

    def test_incorrect_warmup_and_retained_samples_cannot_hide_behind_zero_failure_count(self):
        for group in ("warmup_samples", "samples"):
            with self.subTest(group=group):
                value = payload(self.workload)
                value["workloads"][0][group][-1]["correct"] = False
                with self.assertRaisesRegex(ValueError, "incorrect"):
                    self.parse(value)

    def test_exact_nearest_rank_and_cpu_aggregate_keep_zero_cpu_samples(self):
        value = payload(self.workload)
        row = value["workloads"][0]
        for index, sample in enumerate(row["samples"]):
            sample["elapsed_ns"] = index + 1
            sample["cpu_ns"] = 1000 if index == 199 else 0
        row["metrics"]["cpu_ns"] = 1000
        result = self.parse(value)
        self.assertEqual(result["p95_ns"], 190)
        self.assertEqual(result["cpu_ns"], 1000)
        row["metrics"]["cpu_ns"] = 0
        with self.assertRaisesRegex(ValueError, "aggregate CPU"):
            self.parse(value)

    def test_missing_native_memory_retains_partial_summary_without_a_passing_zero(self):
        for smoke in (False, True):
            with self.subTest(smoke=smoke):
                value = payload(self.workload, smoke=smoke)
                value["workloads"][0]["metrics"].update(peak_rss_bytes=None, native_resource_reason="API unavailable")
                result = self.parse(value, smoke=smoke)
                self.assertIsNone(result["peak_rss_bytes"])
                self.assertIn("peak_rss_bytes", result["unavailable"])
                self.assertEqual(result["peak_heap_bytes"], 8 * collector.MIB)
                self.assertEqual(result["cpu_ns"], value["workloads"][0]["metrics"]["cpu_ns"])

    def test_missing_cpu_retains_partial_summary_without_substitution(self):
        value = payload(self.workload)
        value["workloads"][0]["samples"][3]["cpu_ns"] = None
        value["workloads"][0]["metrics"].update(cpu_ns=None, cpu_reason="CPU clock unavailable")
        result = self.parse(value)
        self.assertIsNone(result["cpu_ns"])
        self.assertIn("samples[3].cpu_ns", result["unavailable"])
        self.assertEqual(result["p95_ns"], 1_000_000)
        self.assertEqual(result["peak_rss_bytes"], 16 * collector.MIB)

    def test_missing_cpu_resolution_retains_known_cpu_and_memory(self):
        value = payload(self.workload)
        value["cpu_timer_resolution_ns"] = None
        result = self.parse(value)
        self.assertIsNone(result["cpu_timer_resolution_ns"])
        self.assertIn("cpu_timer_resolution_ns", result["unavailable"])
        self.assertEqual(result["cpu_ns"], 100_000_000)
        self.assertEqual(result["peak_rss_bytes"], 16 * collector.MIB)

    def test_final_owner_payload_and_live_bounds_are_independent(self):
        for metric, bad in (("bgr_retained_after_drop_bytes", 1),
                            ("bgr_evicted_after_drop_bytes", 1),
                            ("bgr_retained_bytes", 32 * collector.MIB + 1),
                            ("bgr_retained_peak_bytes", 32 * collector.MIB + 1)):
            with self.subTest(metric=metric):
                value = payload(self.workload)
                value["workloads"][0]["metrics"][metric] = bad
                with self.assertRaises(ValueError):
                    self.parse(value)

    def test_boolean_counter_and_missing_sample_are_invalid(self):
        value = payload(self.workload)
        value["workloads"][0]["metrics"]["mapping_copies"] = True
        with self.assertRaisesRegex(ValueError, "bounded integer"):
            self.parse(value)
        value = payload(self.workload)
        value["workloads"][0]["samples"].pop()
        with self.assertRaisesRegex(ValueError, "missing samples"):
            self.parse(value)

    def test_geometry_or_fixture_change_cannot_reuse_row_name(self):
        value = payload(self.workload)
        value["workloads"][0]["width"] = 96
        with self.assertRaisesRegex(ValueError, "workload mismatch"):
            self.parse(value)
        value = payload(self.workload)
        value["fixture_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "fixture_sha256 mismatch"):
            self.parse(value)

    def test_manifest_requires_all_ordered_rows_and_fixed_roles(self):
        for mutation in (lambda rows: rows.pop(), lambda rows: rows.reverse(),
                         lambda rows: rows[4].update(improvement=False)):
            value = manifest()
            mutation(value["workloads"])
            with self.assertRaises(ValueError):
                collector.manifest_identity(value, TARGET, "4.12.0")

    def test_duplicate_keys_and_nonfinite_values_are_refused(self):
        for value in ('{"correct":false,"correct":true}', '{"cpu_ns":NaN}', '{"cpu_ns":Infinity}'):
            with self.assertRaises(ValueError):
                collector.strict_json(value)

    def test_direct_format_case_order_work_and_genuine_active_eviction_are_required(self):
        workload = collector.DIRECT_FORMAT_WORKLOAD
        valid = payload(workload, candidate=True, smoke=True)
        parsed = collector.observations(valid, self.manifest, workload, True)
        self.assertEqual(parsed["metrics"]["backend_find_calls"], 72)
        self.assertEqual(parsed["metrics"]["mapping_copies"], 40)
        mutations = (
            lambda row: row["format_observations"].pop(),
            lambda row: row["format_observations"][0]["cases"].reverse(),
            lambda row: row["format_observations"][0]["transitions"].pop(),
            lambda row: row["format_observations"][0]["cases"][0].update(selected_format="bgra8"),
            lambda row: row["format_observations"][0]["cases"][4]["work"].update(mapping_copies=0),
            lambda row: row["format_observations"][0]["lifetimes"][1]["checkpoints"][1].update(active_evicted_bgr_bytes=0),
            lambda row: row["format_observations"][0]["lifetimes"][1]["checkpoints"][1].update(active_backend_finds=0),
            lambda row: row["format_observations"][0]["lifetimes"][1]["checkpoints"][2].update(active_evicted_bgr_bytes=1),
            lambda row: row["format_observations"][0]["lifetimes"][0]["checkpoints"][-1].update(cache_held_bgr_bytes=1),
            lambda row: row["metrics"].update(mapping_copied_bytes=1),
        )
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                changed = copy.deepcopy(valid)
                mutate(changed["workloads"][0])
                with self.assertRaises(ValueError):
                    collector.observations(changed, self.manifest, workload, True)

    def test_direct_format_declaration_cannot_relabel_the_corpus(self):
        value = manifest()
        row = next(row for row in value["workloads"] if row["id"] == collector.DIRECT_FORMAT_WORKLOAD)
        row["format_contract"]["cases"][0]["expected_mapping_copies"] = 1
        with self.assertRaisesRegex(ValueError, "direct-format declaration"):
            collector.manifest_identity(value, TARGET, "4.12.0")

    def test_native_runtime_needs_actual_absolute_loaded_paths_not_only_matching_hashes(self):
        binding = identity(Path("/binary-root"), "baseline")
        collector.loaded_runtime(manifest(), binding)
        for mutate in (
            lambda value: value.pop("loaded_native_runtime"),
            lambda value: value["loaded_native_runtime"].update(method="hash-only-lookup"),
            lambda value: value["loaded_native_runtime"]["images"][0].update(path="libopencv_core.dylib"),
            lambda value: value["loaded_native_runtime"]["images"][0].update(path="/unobserved/libopencv_core.dylib"),
        ):
            value = manifest()
            mutate(value)
            with self.assertRaises(ValueError):
                collector.loaded_runtime(value, binding)

    def test_windows_loaded_path_comparison_preserves_actual_path_authority(self):
        value = {"target": collector.TARGETS[1], "loaded_native_runtime": {
            "method": "K32EnumProcessModules-GetModuleFileNameW",
            "images": [{"path": r"\\?\C:\opencv\bin\opencv_world4140.dll", "sha256": "e" * 64}]}}
        binding = {"build": {"dependencies": [{"path": r"C:\OPENCV\bin\opencv_world4140.dll", "sha256": "e" * 64}]}}
        collector.loaded_runtime(value, binding)
        binding["build"]["dependencies"][0]["path"] = r"C:\another\opencv_world4140.dll"
        with self.assertRaisesRegex(ValueError, "path/hash"):
            collector.loaded_runtime(value, binding)

    def test_dependency_paths_follow_recorded_target_and_detect_aliases(self):
        cases = (
            (TARGET, "/runtime/libopencv_core.dylib", "runtime/libopencv_core.dylib"),
            (collector.TARGETS[1], r"C:\opencv\opencv_world4140.dll", r"C:opencv_world4140.dll"),
        )
        for target, absolute, relative in cases:
            with self.subTest(target=target):
                build = identity(Path("/binary-root"), "baseline")["build"]
                build["target"] = target
                build["dependencies"] = [{"path": absolute, "sha256": "e" * 64}]
                collector.validate_build(build, target)
                build["dependencies"][0]["path"] = relative
                with self.assertRaisesRegex(ValueError, "absolute"):
                    collector.validate_build(build, target)
        build["dependencies"] = [
            {"path": r"C:\opencv\opencv_world4140.dll", "sha256": "e" * 64},
            {"path": r"\\?\C:\OPENCV\opencv_world4140.dll", "sha256": "e" * 64},
        ]
        with self.assertRaisesRegex(ValueError, "unique"):
            collector.validate_build(build, collector.TARGETS[1])


class AcceptanceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.calibration = parsed_evidence(self.root, "calibration")
        self.profile = collector.profile_from(self.calibration, self.root / "calibration", "reviewer", "fixed controlled host")
        self.comparison = parsed_evidence(self.root, "comparison")

    def test_each_process_guard_wins_over_improved_median(self):
        workload = "selection-k16-dense1080p-keepall"
        rows = self.comparison["observations"]["candidate"][workload]
        for metric in ("p95_ns", "cpu_ns", "peak_rss_bytes", "heap_growth_bytes", "rss_residual_bytes"):
            with self.subTest(metric=metric):
                changed = copy.deepcopy(self.comparison)
                changed["observations"]["candidate"][workload][4][metric] = self.profile["envelopes"][workload]["limits"][metric] + 1
                issues, benefits = collector.compare_evidence(changed, self.profile)
                self.assertTrue(benefits[workload]["p95_ns"]["passed"])
                self.assertTrue(any(metric in issue["reason"] and issue["status"] == "failed" for issue in issues))
        self.assertLess(rows[0]["p95_ns"], self.profile["envelopes"][workload]["limits"]["p95_ns"])

    def test_baseline_process_is_also_subject_to_frozen_ceiling(self):
        workload = "single-small96x64"
        self.comparison["observations"]["baseline"][workload][0]["cpu_ns"] *= 3
        issues, _ = collector.compare_evidence(self.comparison, self.profile)
        self.assertTrue(any(issue["attempt"].startswith("baseline:") and "cpu_ns" in issue["reason"] for issue in issues))

    def test_improvement_must_exceed_full_noise_allowance(self):
        workload = "batch8-cold-full1080p"
        allowance = self.profile["envelopes"][workload]["benefit_allowances"]["cpu_ns"]
        baseline = self.comparison["observations"]["baseline"][workload][0]["cpu_ns"]
        for row in self.comparison["observations"]["candidate"][workload]:
            row["cpu_ns"] = baseline - allowance
        issues, benefits = collector.compare_evidence(self.comparison, self.profile)
        self.assertFalse(benefits[workload]["cpu_ns"]["passed"])
        self.assertTrue(any("benefit" in issue["reason"] for issue in issues))

    def test_live_allowance_never_multiplies_or_applies_to_growth_or_residual(self):
        workload = "batch8-cold-full1080p"
        limits = self.profile["envelopes"][workload]["limits"]
        self.assertEqual(limits["rss_growth_bytes"], collector.MIB)
        self.assertEqual(limits["rss_residual_bytes"], collector.MIB)
        self.assertEqual(limits["heap_growth_bytes"], 256 * 1024)
        self.comparison["observations"]["candidate"][workload][0]["rss_growth_bytes"] = 2 * collector.MIB
        issues, _ = collector.compare_evidence(self.comparison, self.profile)
        self.assertTrue(any("rss_growth_bytes" in issue["reason"] for issue in issues))

    def test_native_cache_allowance_cannot_hide_temporary_rust_scratch(self):
        workload = "batch8-cold-full1080p"
        row = self.comparison["observations"]["candidate"][workload][0]
        row["peak_heap_bytes"] += 16 * collector.MIB
        issues, _ = collector.compare_evidence(self.comparison, self.profile)
        self.assertTrue(any("peak_heap_bytes" in issue["reason"] for issue in issues))

    def test_noisy_calibration_cannot_generate_a_wider_profile(self):
        workload = "selection-k16-dense1080p-keepall"
        self.calibration["observations"]["baseline"][workload][4]["p95_ns"] *= 2
        with self.assertRaisesRegex(ValueError, "noisy calibration"):
            collector.derive_envelopes(self.calibration)

    def test_windows_quantized_guard_uses_absolute_allowance_not_impossible_noise_gate(self):
        self.calibration["plan"]["target"] = "x86_64-pc-windows-msvc"
        workload = "single-small96x64"
        for row in self.calibration["observations"]["baseline"][workload]:
            row["cpu_ns"] = 60_000_000
        envelopes = collector.derive_envelopes(self.calibration)
        self.assertEqual(envelopes[workload]["limits"]["cpu_ns"], 91_250_000)
        for row in self.calibration["observations"]["baseline"][workload]:
            row["cpu_ns"] = 0
        envelopes = collector.derive_envelopes(self.calibration)
        self.assertEqual(envelopes[workload]["limits"]["cpu_ns"], 31_250_000)

    def test_work_failure_is_not_offset_by_latency_benefit(self):
        cases = (("batch8-cold-full1080p", "bgr_conversions", 1760),
                 ("batch8-live-owner-roi320x180", "mapping_copies", 1760),
                 ("selection-k16-dense1080p-keepall", "score_copied_bytes", 1),
                 ("selection-k16-dense1080p-keepall", "score_offsets_scanned", 100_000_000))
        for workload, metric, value in cases:
            with self.subTest(metric=metric):
                changed = copy.deepcopy(self.comparison)
                changed["observations"]["candidate"][workload][0]["metrics"][metric] = value
                issues, _ = collector.compare_evidence(changed, self.profile)
                self.assertTrue(any(issue["status"] == "failed" for issue in issues))

    def test_cross_host_source_or_fixture_cannot_substitute(self):
        for mutate in (lambda value: value["plan"]["host"].update(node="other-host"),
                       lambda value: value["plan"]["binaries"]["baseline"]["build"].update(source_tree="9" * 40),
                       lambda value: value["manifest"].update(fixture_sha256="9" * 64)):
            changed = copy.deepcopy(self.comparison)
            mutate(changed)
            with self.assertRaises(ValueError):
                collector.compare_evidence(changed, self.profile)

    def test_calibration_never_substitutes_for_comparison(self):
        with self.assertRaisesRegex(ValueError, "not a comparison"):
            collector.compare_evidence(self.calibration, self.profile)
        self.comparison["observations"]["baseline"]["single-small96x64"].pop()
        issues, _ = collector.compare_evidence(self.comparison, self.profile)
        self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))

    def test_missing_processes_preserve_every_known_ceiling_and_actual_identity(self):
        workload = "selection-k16-dense1080p-keepall"
        for role in ("baseline", "candidate"):
            for metric, limit in self.profile["envelopes"][workload]["limits"].items():
                with self.subTest(role=role, metric=metric):
                    changed = copy.deepcopy(self.comparison)
                    rows = changed["observations"][role][workload]
                    rows.pop(1)
                    rows[-1][metric] = limit + 1
                    issues, benefits = collector.compare_evidence(changed, self.profile)
                    self.assertTrue(any(issue["status"] == "failed"
                                        and issue["attempt"] == f"{role}:{workload}:4"
                                        and metric in issue["reason"] for issue in issues))
                    self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))
                    self.assertNotIn(workload, benefits)

    def test_independent_work_failures_survive_either_missing_role_cohort(self):
        cases = (("batch8-cold-full1080p", "bgr_conversions", 1760),
                 ("batch8-live-owner-roi320x180", "mapping_copies", 1760),
                 ("selection-k16-dense1080p-keepall", "score_copied_bytes", 1))
        for workload, metric, value in cases:
            for missing_role in ("baseline", "candidate"):
                with self.subTest(metric=metric, missing_role=missing_role):
                    changed = copy.deepcopy(self.comparison)
                    changed["observations"][missing_role][workload].pop(1)
                    changed["observations"]["candidate"][workload][-1]["metrics"][metric] = value
                    issues, benefits = collector.compare_evidence(changed, self.profile)
                    self.assertTrue(any(issue["status"] == "failed"
                                        and issue["attempt"] == f"candidate:{workload}:4" for issue in issues))
                    self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))
                    self.assertNotIn(workload, benefits)

    def test_baseline_relative_offsets_require_five_rows_not_unrelated_observations(self):
        workload = "selection-k16-dense1080p-keepall"
        for missing in ("process", "observation"):
            with self.subTest(missing=missing):
                changed = copy.deepcopy(self.comparison)
                baseline = changed["observations"]["baseline"][workload]
                if missing == "process":
                    baseline.pop()
                else:
                    value = payload(workload)
                    value["cpu_timer_resolution_ns"] = None
                    baseline[0] = {**collector.observations(value, changed["manifest"], workload, False), "process": 0}
                changed["observations"]["candidate"][workload][0]["metrics"]["score_offsets_scanned"] = 100_000_000
                issues, benefits = collector.compare_evidence(changed, self.profile)
                self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))
                self.assertEqual(any(issue["status"] == "failed"
                                     and issue["attempt"] == f"candidate:{workload}:0" for issue in issues),
                                 missing == "observation")
                self.assertNotIn(workload, benefits)
        self.comparison["observations"]["candidate"][workload].pop(1)
        self.comparison["observations"]["candidate"][workload][-1]["metrics"]["score_offsets_scanned"] = 100_000_000
        issues, _ = collector.compare_evidence(self.comparison, self.profile)
        self.assertTrue(any(issue["status"] == "failed"
                            and issue["attempt"] == f"candidate:{workload}:4" for issue in issues))

    def test_missing_cohorts_cannot_claim_median_benefits(self):
        workload = "batch8-cold-full1080p"
        for role in ("baseline", "candidate"):
            for remaining in (0, 4):
                with self.subTest(role=role, remaining=remaining):
                    changed = copy.deepcopy(self.comparison)
                    for row in changed["observations"]["candidate"][workload]:
                        for metric in ("p95_ns", "cpu_ns"):
                            row[metric] = changed["observations"]["baseline"][workload][0][metric]
                    changed["observations"][role][workload] = changed["observations"][role][workload][:remaining]
                    issues, benefits = collector.compare_evidence(changed, self.profile)
                    self.assertTrue(issues)
                    self.assertTrue(all(issue["status"] == "incomplete" for issue in issues))
                    self.assertNotIn(workload, benefits)

    def test_known_benefit_failures_survive_unavailable_rss_or_cpu_resolution(self):
        workload = "selection-k16-dense1080p-keepall"
        for metric in ("p95_ns", "cpu_ns"):
            for missing in ("rss", "cpu-resolution"):
                for role in ("baseline", "candidate"):
                    with self.subTest(metric=metric, missing=missing, role=role):
                        changed = copy.deepcopy(self.comparison)
                        value = payload(workload, candidate=role == "candidate")
                        if missing == "rss":
                            value["workloads"][0]["metrics"].update(
                                peak_rss_bytes=None, native_resource_reason="API unavailable")
                        else:
                            value["cpu_timer_resolution_ns"] = None
                        changed["observations"][role][workload][0] = {
                            **collector.observations(value, changed["manifest"], workload, False), "process": 0}
                        baseline = changed["observations"]["baseline"][workload][0][metric]
                        allowance = self.profile["envelopes"][workload]["benefit_allowances"][metric]
                        for row in changed["observations"]["candidate"][workload]:
                            row[metric] = baseline - allowance
                        issues, benefits = collector.compare_evidence(changed, self.profile)
                        failures = [issue for issue in issues if issue["status"] == "failed"]
                        self.assertEqual(len(failures), 1)
                        self.assertIn(f"{metric} benefit", failures[0]["reason"])
                        self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))
                        self.assertNotIn(workload, benefits)

    def test_known_p95_benefit_failure_does_not_judge_unavailable_cpu(self):
        workload = "selection-k16-dense1080p-keepall"
        value = payload(workload, candidate=True)
        value["workloads"][0]["samples"][3]["cpu_ns"] = None
        value["workloads"][0]["metrics"].update(cpu_ns=None, cpu_reason="CPU clock unavailable")
        candidates = self.comparison["observations"]["candidate"][workload]
        candidates[0] = {**collector.observations(value, self.comparison["manifest"], workload, False), "process": 0}
        baseline = self.comparison["observations"]["baseline"][workload][0]
        allowance = self.profile["envelopes"][workload]["benefit_allowances"]["p95_ns"]
        for row in candidates:
            row["p95_ns"] = baseline["p95_ns"] - allowance
            if row["cpu_ns"] is not None:
                row["cpu_ns"] = baseline["cpu_ns"]
        issues, benefits = collector.compare_evidence(self.comparison, self.profile)
        failures = [issue for issue in issues if issue["status"] == "failed"]
        self.assertEqual(len(failures), 1)
        self.assertIn("p95_ns benefit", failures[0]["reason"])
        self.assertFalse(any("cpu_ns benefit" in issue["reason"] for issue in issues))
        self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))
        self.assertNotIn(workload, benefits)

    def test_partial_process_reports_incomplete_unless_an_available_fact_fails(self):
        calibration, comparison = self.root / "calibration", self.root / "comparison"
        persist_campaign(calibration, "calibration")
        profile = collector.profile_from(collector.evaluate(calibration), calibration, "reviewer", "controlled host")
        specification = persist_campaign(comparison, "comparison", profile=profile)
        workload = "selection-k16-dense1080p-keepall"
        task = next(task for task in specification["schedule"]
                    if task["role"] == "candidate" and task["workload"] == workload and task["process"] == 4)
        path = comparison / "attempts" / f"{task['id']}.json"
        original = collector.load(path)
        cases = (("cpu-resolution", "cpu_timer_resolution_ns", "peak_rss_bytes"),
                 ("cpu", "cpu_ns", "score_copied_bytes"),
                 ("memory", "peak_heap_bytes", "peak_rss_bytes"))
        for missing, unavailable, failure in cases:
            for known_failure in (False, True):
                with self.subTest(missing=missing, known_failure=known_failure):
                    attempt = copy.deepcopy(original)
                    value = collector.strict_json(attempt["record"]["stdout"])
                    row = value["workloads"][0]
                    if missing == "cpu-resolution":
                        value["cpu_timer_resolution_ns"] = None
                    elif missing == "cpu":
                        row["samples"][3]["cpu_ns"] = None
                        row["metrics"].update(cpu_ns=None, cpu_reason="CPU clock unavailable")
                    else:
                        row["metrics"].update(peak_heap_bytes=None, native_resource_reason="API unavailable")
                    if known_failure:
                        row["metrics"][failure] = (1 if failure == "score_copied_bytes"
                                                   else profile["envelopes"][workload]["limits"][failure] + 1)
                    attempt["record"]["stdout"] = json.dumps(value)
                    path.write_bytes(collector.document(attempt))
                    result = collector.report(comparison)
                    self.assertEqual(result["status"], "failed" if known_failure else "incomplete")
                    self.assertFalse(result["performance_qualified"])
                    self.assertTrue(any(issue["status"] == "incomplete" for issue in result["issues"]))
                    self.assertEqual(any(issue["status"] == "failed"
                                         and issue["attempt"] == f"candidate:{workload}:4"
                                         for issue in result["issues"]), known_failure)
                    summary = result["process_observations"]["candidate"][workload][-1]
                    self.assertEqual(summary["process"], 4)
                    self.assertIsNone(summary[unavailable])
                    self.assertTrue(summary["unavailable"])
                    self.assertNotIn(workload, result["benefits"])


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def mutate_attempt(self, directory, index, mutate):
        path = directory / "attempts" / f"{index:04d}.json"
        value = collector.load(path)
        mutate(value)
        path.write_bytes(collector.document(value))

    def test_valid_json_cannot_cover_exit_timeout_output_or_cleanup_failure(self):
        directory = self.root / "smoke"
        persist_campaign(directory, "smoke", smoke=True)
        path = directory / "attempts/0001.json"
        original = path.read_bytes()
        for field, value in (("exit_code", 1), ("timed_out", True), ("output_limited", True),
                             ("cleanup_ok", False), ("stderr", None), ("duration_seconds", None)):
            with self.subTest(field=field):
                path.write_bytes(original)
                self.mutate_attempt(directory, 1, lambda attempt: attempt["record"].update({field: value}))
                result = collector.report(directory)
                self.assertEqual(result["status"], "failed")
                self.assertFalse(result["performance_qualified"])
                self.assertEqual(len(result["attempts"]), 2)

    def test_smoke_report_cannot_claim_either_target_qualification(self):
        directory = self.root / "smoke"
        persist_campaign(directory, "smoke", smoke=True)
        result = collector.report(directory)
        self.assertEqual(result["status"], "passed")
        self.assertFalse(result["performance_qualified"])
        self.assertEqual(result["targets"]["x86_64-pc-windows-msvc"], "not-run")
        self.assertEqual(result["whole_change_acceptance"], "incomplete")

    def test_recorded_binary_paths_follow_the_producer_target(self):
        specification = plan(self.root, "smoke")
        collector.validate_plan(specification)
        windows = collector.TARGETS[1]
        specification["target"] = windows
        specification["host"]["target"] = windows
        binding = specification["binaries"]["baseline"]
        binding["path"] = r"D:\recorded\benchmark.exe"
        binding["build"]["target"] = windows
        binding["build"]["dependencies"] = [
            {"path": r"C:\opencv\opencv_world4140.dll", "sha256": "e" * 64}]
        collector.validate_plan(specification)
        binding["path"] = r"D:benchmark.exe"
        with self.assertRaisesRegex(ValueError, "absolute"):
            collector.validate_plan(specification)

    def test_missing_attempt_remains_not_run_and_no_retry_is_accepted(self):
        directory = self.root / "smoke"
        persist_campaign(directory, "smoke", smoke=True)
        (directory / "attempts/0001.json").unlink()
        result = collector.report(directory)
        self.assertEqual(result["status"], "incomplete")
        self.assertTrue(any(issue["status"] == "not-run" for issue in result["issues"]))
        collector.save(directory / "attempts/9999.json", {})
        with self.assertRaisesRegex(ValueError, "hidden retry"):
            collector.report(directory)

    def test_started_attempt_is_incomplete_and_never_started_remains_not_run(self):
        directory = self.root / "calibration"
        specification = persist_campaign(directory, "calibration")
        started = directory / "started"
        started.mkdir()
        attempt_path = directory / "attempts/0001.json"
        attempt = collector.load(attempt_path)
        attempt.update(identity_before=None, identity_after=None, record=None, started_unix_ns=1)
        collector.save(started / attempt_path.name, attempt)
        marker_hash = collector.sha256(started / attempt_path.name)
        attempt_path.unlink()
        (directory / "attempts/0002.json").unlink()
        result = collector.report(directory)
        issues = {issue["attempt"].split(":")[0]: issue for issue in result["issues"]}
        self.assertEqual(result["status"], "incomplete")
        self.assertEqual(issues["0001"]["status"], "incomplete")
        self.assertEqual(issues["0001"]["file"], "started/0001.json")
        self.assertEqual(issues["0002"]["status"], "not-run")
        workload = specification["schedule"][1]["workload"]
        self.assertEqual([row["process"] for row in result["process_observations"]["baseline"][workload]],
                         [2, 3, 4])
        self.assertIn({"file": "started/0001.json", "sha256": marker_hash}, result["attempts"])
        self.assertEqual(collector.sha256(started / attempt_path.name), marker_hash)

    def test_unexpected_started_marker_is_rejected_even_with_completed_attempts(self):
        directory = self.root / "smoke"
        persist_campaign(directory, "smoke", smoke=True)
        (directory / "started").mkdir()
        collector.save(directory / "started/9999.json", {})
        with self.assertRaisesRegex(ValueError, "hidden retry"):
            collector.report(directory)

    def test_damaged_attempt_is_attributed_without_losing_other_results_or_hashes(self):
        for damage in ("truncated", "encoding", "non-object", "unreadable"):
            with self.subTest(damage=damage):
                directory = self.root / damage
                specification = persist_campaign(directory, "calibration")
                plan_bytes = (directory / "plan.json").read_bytes()
                path = directory / "attempts/0001.json"
                intact_path = directory / "attempts/0002.json"
                intact_bytes, intact_hash = intact_path.read_bytes(), collector.sha256(intact_path)
                if damage == "unreadable":
                    path.unlink()
                    path.mkdir()
                    damaged_hash = None
                else:
                    path.write_bytes({"truncated": b'{"task":', "encoding": b"\xff",
                                      "non-object": b"[]"}[damage])
                    damaged_hash = collector.sha256(path)
                result = collector.report(directory)
                self.assertEqual(result["status"], "failed")
                issue = next(issue for issue in result["issues"] if issue["attempt"].startswith("0001:"))
                self.assertEqual(issue["status"], "failed")
                self.assertEqual(issue["file"], "attempts/0001.json")
                self.assertIn("attempts/0001.json", issue["reason"])
                self.assertEqual(len(result["attempts"]), len(specification["schedule"]))
                self.assertIn({"file": "0001.json", "sha256": damaged_hash}, result["attempts"])
                self.assertIn({"file": "0002.json", "sha256": intact_hash}, result["attempts"])
                workload = specification["schedule"][1]["workload"]
                self.assertEqual([row["process"] for row in result["process_observations"]["baseline"][workload]],
                                 [1, 2, 3, 4])
                self.assertEqual(intact_path.read_bytes(), intact_bytes)
                self.assertEqual((directory / "plan.json").read_bytes(), plan_bytes)
                self.assertEqual(result["plan_sha256"], collector.sha256(directory / "plan.json"))
                self.assertEqual(specification["collector"], collector.collector_identity())

    def test_damaged_started_marker_is_failed_not_never_started(self):
        directory = self.root / "smoke"
        persist_campaign(directory, "smoke", smoke=True)
        (directory / "attempts/0001.json").unlink()
        (directory / "started").mkdir()
        marker = directory / "started/0001.json"
        marker.write_bytes(b'{"task":')
        result = collector.report(directory)
        issue = next(issue for issue in result["issues"] if issue["attempt"].startswith("0001:"))
        self.assertEqual(result["status"], "failed")
        self.assertEqual(issue["status"], "failed")
        self.assertEqual(issue["file"], "started/0001.json")
        self.assertIn("JSONDecodeError", issue["reason"])
        self.assertIn({"file": "started/0001.json", "sha256": collector.sha256(marker)}, result["attempts"])

    def test_smoke_retains_unavailable_measurements_without_qualification(self):
        directory = self.root / "smoke"
        persist_campaign(directory, "smoke", smoke=True)
        value = payload("single-small96x64", smoke=True)
        value["cpu_timer_resolution_ns"] = None
        self.mutate_attempt(directory, 1, lambda attempt: attempt["record"].update(stdout=json.dumps(value)))
        result = collector.report(directory)
        self.assertEqual(result["status"], "passed")
        self.assertFalse(result["performance_qualified"])
        self.assertFalse(result["issues"])
        summary = result["process_observations"]["baseline"]["single-small96x64"][0]
        self.assertEqual(summary["process"], 0)
        self.assertIn("cpu_timer_resolution_ns", summary["unavailable"])
        self.assertIsNone(summary["cpu_timer_resolution_ns"])
        self.assertEqual(summary["peak_rss_bytes"], 16 * collector.MIB)

    def test_interruption_propagates_with_only_observed_process_facts(self):
        cases = (("before", KeyboardInterrupt), ("runner", KeyboardInterrupt),
                 ("after", KeyboardInterrupt), ("runner", SystemExit))
        for index, (stage, exception_type) in enumerate(cases):
            with self.subTest(stage=stage, exception=exception_type.__name__):
                destination = self.root / f"interrupted-{index}"
                args = collector.parser().parse_args([
                    "smoke", "--protocol-version", "1", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "build.json",
                    "--output", str(destination), "--workload", "single-small96x64"])
                interruption = exception_type("collection stopped")
                record = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": False,
                          "launch_error": None, "stdout": json.dumps(manifest()), "stderr": "retained diagnostic",
                          "duration_seconds": 0.1}
                checks = [interruption] if stage == "before" else [None, interruption] if stage == "after" else [None]
                binding = identity(self.root, "baseline")
                with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET, "node": "host"}),
                      mock.patch.object(collector, "binary_identity", return_value=binding),
                      mock.patch.object(collector, "check_files", side_effect=checks),
                      mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
                      mock.patch.object(collector, "run_process", return_value=record,
                                        side_effect=interruption if stage == "runner" else None) as launch):
                    with self.assertRaises(exception_type) as raised:
                        collector.collect(args, "smoke")
                self.assertIs(raised.exception, interruption)
                self.assertEqual(launch.call_count, 0 if stage == "before" else 1)
                attempt_path = destination / "attempts/0000.json"
                attempt = collector.load(attempt_path)
                self.assertEqual(attempt["interruption"], exception_type.__name__)
                self.assertEqual(attempt["identity_before"], None if stage == "before" else binding)
                self.assertIsNone(attempt["identity_after"])
                self.assertIsNone(attempt["identity_error"])
                if stage == "after":
                    self.assertEqual(attempt["record"], record)
                else:
                    self.assertIsNone(attempt["record"])
                self.assertNotIn("cleanup_ok", attempt)
                self.assertTrue((destination / "started/0000.json").is_file())
                self.assertFalse((destination / "started/0001.json").exists())
                self.assertFalse((destination / "attempts/0001.json").exists())
                self.assertFalse((destination / "report.json").exists())
                result = collector.report(destination)
                self.assertEqual(result["status"], "failed")
                issues = {issue["attempt"].split(":")[0]: issue for issue in result["issues"]}
                self.assertEqual(issues["0000"]["status"], "failed")
                self.assertIn(exception_type.__name__, issues["0000"]["reason"])
                self.assertEqual(issues["0001"]["status"], "not-run")
                self.assertIn({"file": "0000.json", "sha256": collector.sha256(attempt_path)}, result["attempts"])
                self.assertEqual(collector.load(destination / "plan.json")["collector"], collector.collector_identity())

    def test_interruption_survives_real_evidence_write_failure(self):
        destination = self.root / "interrupted"
        args = collector.parser().parse_args([
            "smoke", "--protocol-version", "1", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "build.json",
            "--output", str(destination), "--workload", "single-small96x64"])
        interruption = KeyboardInterrupt("collection stopped")

        def interrupt_with_damaged_destination(*args, **kwargs):
            (destination / "attempts/0000.json").mkdir()
            raise interruption

        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET, "node": "host"}),
              mock.patch.object(collector, "binary_identity", return_value=identity(self.root, "baseline")),
              mock.patch.object(collector, "check_files"),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector, "run_process", side_effect=interrupt_with_damaged_destination)):
            with self.assertRaises(KeyboardInterrupt) as raised:
                collector.collect(args, "smoke")
        self.assertIs(raised.exception, interruption)
        self.assertTrue((destination / "attempts/0000.json").is_dir())
        started = collector.load(destination / "started/0000.json")
        self.assertIsNone(started["record"])
        self.assertIsNone(started["identity_after"])
        self.assertFalse((destination / "attempts/0001.json").exists())
        self.assertFalse((destination / "report.json").exists())
        result = collector.report(destination)
        issues = {issue["attempt"].split(":")[0]: issue for issue in result["issues"]}
        self.assertEqual(result["status"], "failed")
        self.assertEqual(issues["0000"]["status"], "failed")
        self.assertEqual(issues["0000"]["file"], "attempts/0000.json")
        self.assertEqual(issues["0001"]["status"], "not-run")
        self.assertIn({"file": "0000.json", "sha256": None}, result["attempts"])

    def test_numeric_profile_is_rederived_and_raw_calibration_hashes_are_bound(self):
        directory = self.root / "calibration"
        persist_campaign(directory, "calibration")
        evidence = collector.evaluate(directory)
        profile = collector.profile_from(evidence, directory, "reviewer", "controlled baseline five")
        collector.validate_profile(profile)
        widened = copy.deepcopy(profile)
        widened["envelopes"]["single-small96x64"]["limits"]["p95_ns"] += 1
        with self.assertRaisesRegex(ValueError, "differ from frozen calibration"):
            collector.validate_profile(widened)
        self.mutate_attempt(directory, 1, lambda attempt: attempt["record"].update(stderr="retained native warning"))
        with self.assertRaisesRegex(ValueError, "differ from frozen calibration"):
            collector.validate_profile(profile)

    def test_comparison_revalidates_profile_and_does_not_modify_calibration(self):
        calibration, comparison = self.root / "calibration", self.root / "comparison"
        persist_campaign(calibration, "calibration")
        profile = collector.profile_from(collector.evaluate(calibration), calibration, "reviewer", "controlled host")
        before = {path.relative_to(calibration): collector.sha256(path) for path in calibration.rglob("*.json")}
        persist_campaign(comparison, "comparison", profile=profile)
        result = collector.report(comparison)
        self.assertTrue(result["performance_qualified"])
        self.assertEqual(before, {path.relative_to(calibration): collector.sha256(path) for path in calibration.rglob("*.json")})
        changed = collector.load(comparison / "profile.json")
        changed["envelopes"]["single-small96x64"]["limits"]["p95_ns"] += 1
        (comparison / "profile.json").write_bytes(collector.document(changed))
        result = collector.report(comparison)
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["performance_qualified"])

    def test_attempt_cannot_claim_other_binary_or_sampling_command(self):
        directory = self.root / "smoke"
        persist_campaign(directory, "smoke", smoke=True)
        self.mutate_attempt(directory, 1, lambda attempt: attempt["argv"].append("--workload=smaller"))
        self.assertEqual(collector.report(directory)["status"], "failed")
        directory = self.root / "second"
        persist_campaign(directory, "smoke", smoke=True)
        self.mutate_attempt(directory, 1, lambda attempt: attempt["identity_after"].update(sha256="9" * 64))
        self.assertEqual(collector.report(directory)["status"], "failed")

    def test_prebuilt_source_attestation_rejects_changed_binary_or_dependency(self):
        binary, dependency, metadata = self.root / "binary", self.root / "opencv", self.root / "build.json"
        binary.write_bytes(b"explicit prebuilt bytes")
        dependency.write_bytes(b"native library bytes")
        build = identity(self.root, "baseline")["build"]
        build["target"] = NATIVE_TARGET
        build["binary_sha256"] = collector.sha256(binary)
        build["dependencies"][0]["path"] = str(dependency)
        build["dependencies"][0]["sha256"] = collector.sha256(dependency)
        collector.save(metadata, build)
        binding = collector.binary_identity(binary, metadata, NATIVE_TARGET)
        dependency.write_bytes(b"another native library")
        with self.assertRaisesRegex(ValueError, "native dependency changed"):
            collector.check_files(binding)
        binary.write_bytes(b"rebuilt in place")
        with self.assertRaisesRegex(ValueError, "binary does not match"):
            collector.binary_identity(binary, metadata, NATIVE_TARGET)

    def test_attestation_preserves_existing_metadata_and_probe_without_launch(self):
        binary = self.root / "binary"
        binary.write_bytes(b"explicit executable bytes")
        for index, suffix in enumerate(("", ".probe.json")):
            with self.subTest(existing_suffix=suffix):
                metadata = self.root / f"existing-{index}.json"
                probe = Path(str(metadata) + ".probe.json")
                existing = Path(str(metadata) + suffix)
                original = b"immutable prior evidence\n"
                existing.write_bytes(original)
                args = collector.parser().parse_args([
                    "attest", "--protocol-version", "2", "--build-mode", "normal",
                    "--target", TARGET, "--binary", str(binary),
                    "--build-argv-json", '["cargo","bench"]', "--output", str(metadata)])
                with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
                      mock.patch.object(collector.shutil, "which", return_value=sys.executable),
                      mock.patch.object(collector, "run_process") as launch):
                    with self.assertRaises(ValueError):
                        collector.attest(args)
                launch.assert_not_called()
                self.assertEqual(existing.read_bytes(), original)
                self.assertFalse((probe if existing == metadata else metadata).exists())

    def test_failed_attestation_probe_is_preserved_without_retry_or_metadata(self):
        binary, metadata = self.root / "binary", self.root / "failed-build.json"
        binary.write_bytes(b"explicit executable bytes")
        args = collector.parser().parse_args([
            "attest", "--protocol-version", "1", "--target", TARGET, "--binary", str(binary), "--build-argv-json", '["cargo","bench"]',
            "--output", str(metadata)])
        record = {"exit_code": 1, "timed_out": False, "output_limited": False, "cleanup_ok": True,
                  "launch_error": None, "duration_seconds": 0.1, "stdout": "partial observation", "stderr": "failure"}
        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "run_process", return_value=record) as launch):
            with self.assertRaisesRegex(ValueError, "attestation command failed"):
                collector.attest(args)
        self.assertEqual(launch.call_count, 1)
        self.assertFalse(metadata.exists())
        self.assertEqual(collector.load(str(metadata) + ".probe.json")["commands"][0]["record"], record)

    def test_attestation_preserves_completed_probe_on_later_launch_exception(self):
        binary, metadata = self.root / "binary", self.root / "interrupted-build.json"
        binary.write_bytes(b"explicit executable bytes")
        args = collector.parser().parse_args([
            "attest", "--protocol-version", "1", "--target", TARGET, "--binary", str(binary), "--build-argv-json", '["cargo","bench"]',
            "--output", str(metadata)])
        record = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
                  "launch_error": None, "duration_seconds": 0.1, "stdout": "completed probe", "stderr": ""}
        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "run_process", side_effect=[record, ValueError("launch rejected")])):
            with self.assertRaisesRegex(ValueError, "launch rejected"):
                collector.attest(args)
        self.assertFalse(metadata.exists())
        self.assertEqual(collector.load(str(metadata) + ".probe.json")["commands"][0]["record"], record)

    def test_exclusive_output_write_preserves_existing_evidence(self):
        path = self.root / "profile.json"
        collector.save(path, {"reviewed": True})
        original = path.read_bytes()
        with self.assertRaises(FileExistsError):
            collector.save(path, {"reviewed": False})
        self.assertEqual(path.read_bytes(), original)

    def test_comparison_schedule_alternates_roles_with_distinct_process_slots(self):
        jobs = collector.schedule("comparison", ["single-small96x64"])[2:]
        self.assertEqual([(job["role"], job["process"]) for job in jobs],
                         [(role, index) for index in range(5) for role in ("baseline", "candidate")])
        self.assertEqual(len({job["id"] for job in jobs}), 10)

    def test_invalid_manifest_stops_measurement_but_preserves_attempt_and_unrun_rows(self):
        destination = self.root / "collection"
        args = collector.parser().parse_args([
            "smoke", "--protocol-version", "1", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "build.json",
            "--output", str(destination), "--workload", "single-small96x64"])
        wrong = manifest()
        wrong["workloads"].pop()
        record = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
                  "launch_error": None, "stdout": json.dumps(wrong), "stderr": "retained diagnostic",
                  "duration_seconds": 0.1}
        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET, "node": "host"}),
              mock.patch.object(collector, "binary_identity", return_value=identity(self.root, "baseline")),
              mock.patch.object(collector, "check_files"),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector, "run_process", return_value=record) as launch):
            result = collector.collect(args, "smoke")
        self.assertEqual(launch.call_count, 1)
        self.assertEqual(result["status"], "failed")
        self.assertTrue(any(issue["status"] == "not-run" for issue in result["issues"]))
        self.assertEqual(collector.load(destination / "attempts/0000.json")["record"]["stderr"],
                         "retained diagnostic")

    def test_legacy_profiles_cannot_start_new_macos_qualification_even_when_pinned(self):
        calibration = self.root / "calibration"
        persist_campaign(calibration, "calibration")
        profile = collector.profile_from(collector.evaluate(calibration), calibration, "reviewer", "fixed host")
        profile_path = self.root / "profile.json"
        collector.save(profile_path, profile)
        destination = calibration / "candidate-results"
        args = collector.parser().parse_args([
            "compare", "--protocol-version", "1", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "base.json",
            "--candidate", "candidate", "--candidate-metadata", "candidate.json", "--exclusive-host",
            "--profile", str(profile_path), "--profile-sha256", collector.sha256(profile_path),
            "--output", str(destination)])
        with (mock.patch.object(collector, "host_identity", return_value=profile["host"]),
              mock.patch.object(collector, "environment", return_value=profile["environment"]),
              mock.patch.object(collector, "binary_identity",
                                side_effect=lambda binary, metadata, target: identity(self.root, binary)),
              mock.patch.object(collector, "run_process") as launch):
            with self.assertRaises(ValueError):
                collector.collect(args, "comparison")
            args.profile_sha256 = "0" * 64
            with self.assertRaises(ValueError):
                collector.collect(args, "comparison")
        launch.assert_not_called()
        self.assertFalse(destination.exists())

    def test_cli_never_accepts_comparison_without_a_pinned_profile(self):
        with mock.patch("sys.stderr"), self.assertRaises(SystemExit):
            collector.parser().parse_args(["compare", "--target", TARGET, "--baseline", "base",
                                           "--baseline-metadata", "base.json", "--candidate", "candidate",
                                           "--candidate-metadata", "candidate.json", "--output", "out", "--exclusive-host"])


class BoundedExecutionTests(unittest.TestCase):
    def test_timeout_is_not_a_valid_observation_and_cleanup_is_owned(self):
        with tempfile.TemporaryDirectory() as directory:
            result = collector.run_process([sys.executable, "-c", "import time; time.sleep(10)"],
                                           cwd=Path(directory).resolve(), env=dict(os.environ),
                                           timeout_seconds=0.1, output_limit_bytes=4096, cleanup_seconds=5)
        self.assertTrue(result["timed_out"])
        self.assertTrue(result["cleanup_ok"])
        self.assertFalse(collector.successful(result))

    def test_output_limit_retains_failure_instead_of_parsing_a_clean_prefix(self):
        with tempfile.TemporaryDirectory() as directory:
            result = collector.run_process([sys.executable, "-c", "import os; os.write(1, b'x' * 20000)"],
                                           cwd=Path(directory).resolve(), env=dict(os.environ),
                                           timeout_seconds=5, output_limit_bytes=1024, cleanup_seconds=5)
        self.assertTrue(result["output_limited"])
        self.assertLessEqual(len(result["stdout"].encode()) + len(result["stderr"].encode()), 1024)
        self.assertFalse(collector.successful(result))

    def test_smoke_enforces_child_and_campaign_bounds_without_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "smoke"
            args = collector.parser().parse_args([
                "smoke", "--protocol-version", "1", "--target", TARGET, "--baseline", "binary", "--baseline-metadata", "build.json",
                "--output", str(output), "--workload", "single-small96x64"])
            binding = identity(root, "baseline")
            record = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
                      "launch_error": None, "duration_seconds": 1.0, "stdout": json.dumps(manifest()), "stderr": ""}
            with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
                  mock.patch.object(collector, "binary_identity", return_value=binding),
                  mock.patch.object(collector, "check_files"),
                  mock.patch.object(collector, "loaded_runtime"),
                  mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
                  mock.patch.object(collector.time, "monotonic", side_effect=[0, 0, 0, 901]),
                  mock.patch.object(collector, "run_process", return_value=record) as launch):
                result = collector.collect(args, "smoke")
            self.assertEqual(launch.call_count, 1)
            self.assertEqual(launch.call_args.kwargs["timeout_seconds"], 90)
            self.assertEqual(launch.call_args.kwargs["output_limit_bytes"], 8 * collector.MIB)
            self.assertEqual(result["status"], "incomplete")
            self.assertTrue(any(issue["status"] == "not-run" for issue in result["issues"]))
            self.assertEqual(collector.load(output / "plan.json")["execution_limits"], collector.SMOKE_LIMITS)
            self.assertEqual([path.name for path in (output / "attempts").glob("*.json")], ["0000.json"])

    def test_smoke_execution_bounds_cannot_be_relaxed_in_retained_plan(self):
        with tempfile.TemporaryDirectory() as directory:
            value = plan(Path(directory).resolve(), "smoke")
            value["execution_limits"] = {**collector.SMOKE_LIMITS, "child_timeout_seconds": 300}
            with self.assertRaisesRegex(ValueError, "bounded smoke"):
                collector.validate_plan(value)


def v2_fields(build_mode):
    return {"format_version": 2, "build_mode": build_mode,
            "global_allocator": "system" if build_mode == "normal" else "accounting-over-system",
            "opencv_parallel_threads": {"method": collector.OPENCV_THREAD_METHOD,
                                        "before": {"framework": "fixture-runtime", "threads": 4, "reason": None},
                                        "after": {"framework": "fixture-runtime", "threads": 4, "reason": None},
                                        "changed": False},
            "unavailable": ({"reason": "work and heap counters are not compiled",
                             "metrics": list(collector.NORMAL_UNAVAILABLE_METRICS),
                             "workloads": list(collector.NORMAL_UNAVAILABLE_WORKLOADS)}
                            if build_mode == "normal" else {"reason": None, "metrics": [], "workloads": []})}


def v2_manifest(build_mode="normal"):
    value = manifest()
    value.update(v2_fields(build_mode))
    value["workloads"] = [row for row in value["workloads"] if row["id"] in collector.definitions_for(build_mode)]
    value["fixture"] = {"license": "repository-first-party", "generator": "deterministic-test-fixture",
                        "direct_formats": {"helper_source_sha256": "1" * 64, "fixture_source_sha256": "2" * 64},
                        "watcher": {"helper_source_sha256": "3" * 64, "generator_source_sha256": "4" * 64}}
    return value


def v2_payload(workload, build_mode="normal", *, candidate=False, smoke=False):
    value = payload(workload, candidate=candidate, smoke=smoke)
    declaration = v2_manifest(build_mode)
    value.update(v2_fields(build_mode), fixture=declaration["fixture"])
    metrics = value["workloads"][0]["metrics"]
    metrics.setdefault("backend_find_calls", metrics["iterations"])
    metrics.update(logical_mapped_bytes=0,
                   backend_find_failures=0, peak_active_finds=1,
                   peak_active_finds_scope="process-high-water", final_owner_correct=True,
                   native_allocation_scope="process RSS only", mapping_owner_live_before_drop=True,
                   private_after_bytes=None, private_reason="not available on fixture target",
                   footprint_after_bytes=12 * collector.MIB, footprint_reason=None)
    if build_mode == "normal":
        metrics.update({name: None for name in collector.NORMAL_UNAVAILABLE_METRICS})
    return value


def process_record(value):
    return {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
            "launch_error": None, "duration_seconds": 0.1,
            "stdout": json.dumps(value) if not isinstance(value, str) else value, "stderr": ""}


def v2_identity(root, build_mode="normal", role="baseline"):
    binding = identity(root, role)
    build = binding["build"]
    declaration = v2_manifest(build_mode)
    build.update(protocol_version=2, build_mode=build_mode, source_status_porcelain="",
                 benchmark_manifest=collector.manifest_identity(declaration, TARGET, "4.12.0", 2, build_mode))
    commands = [
        ([binding["path"], "--protocol-version", "2", "--list"], process_record(declaration)),
        (["/git", "rev-parse", "HEAD"], process_record(build["source_revision"] + "\n")),
        (["/git", "rev-parse", "HEAD^{tree}"], process_record(build["source_tree"] + "\n")),
        (["/git", "status", "--porcelain"], process_record("")),
        (["/rustc", "-vV"], process_record(build["rustc"])),
    ]
    build["provenance_commands"] = [{"argv": argv, "record": record} for argv, record in commands]
    return binding


def timing_identity(binding):
    binding = copy.deepcopy(binding)
    build = binding["build"]
    probe = build["provenance_commands"][0]["record"]
    value = collector.strict_json(probe["stdout"])
    value.update(timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=False)
    probe["stdout"] = json.dumps(value)
    build["benchmark_manifest"] = collector.manifest_identity(
        value, build["target"], build["opencv_version"], build["protocol_version"], build["build_mode"])
    return binding


def v2_plan(root, mode, build_mode="normal"):
    value = plan(root, "comparison" if mode == "comparison" else "calibration")
    rows = list(collector.definitions_for(build_mode))
    binaries = {"baseline": v2_identity(root, build_mode)}
    if mode == "control":
        binaries["control"] = copy.deepcopy(binaries["baseline"])
    if mode == "comparison":
        binaries["candidate"] = v2_identity(root, build_mode, "candidate")
    jobs = collector.schedule(mode, rows, 2)
    value.update(format_version=2, mode=mode, build_mode=build_mode,
                 policy=copy.deepcopy(collector.V2_POLICY), binaries=binaries, workloads=rows, schedule=jobs,
                 execution_limits=collector.execution_limits(mode, len(jobs)),
                 method_review={"path": str(root / "method.json"), "sha256": "9" * 64},
                 profile_sha256="8" * 64)
    return value


def v2_parsed_evidence(root, mode, build_mode="normal"):
    # Synthetic scalar cohorts exercise arithmetic only, never closure admission.
    specification = v2_plan(root, mode, build_mode)
    frozen = collector.manifest_identity(v2_manifest(build_mode), TARGET, "4.12.0", 2, build_mode)
    cohorts = {}
    for role in specification["binaries"]:
        cohorts[role] = {}
        for workload in specification["workloads"]:
            cohorts[role][workload] = [
                {**collector.observations(v2_payload(workload, build_mode, candidate=role == "candidate"),
                                          frozen, workload, False),
                 "process": process,
                 "position": (process % 2 if role == "baseline" else 1 - process % 2)
                             if mode in ("control", "comparison") else 0}
                for process in range(6)]
    token = {"calibration": "1", "control": "2", "comparison": "3"}[mode]
    if mode == "control":
        specification["calibration"] = {
            "path": str(root / "calibration"), "plan_sha256": "1" * 64,
            "attempts": [{"file": "0000.json", "sha256": "1" * 64}]}
    return {"plan": specification, "plan_sha256": token * 64,
            "attempts": [{"file": "0000.json", "sha256": token * 64}],
            "manifest": frozen, "observations": cohorts, "issues": [],
            "native_threads": {role: collector.effective_threads(v2_manifest(build_mode))
                               for role in specification["binaries"]}}


def v2_numeric_profile(evidence):
    specification = evidence["plan"]
    return {"format_version": specification["format_version"],
            **({"workloads": specification["workloads"]} if specification["format_version"] >= 3 else {}),
            **({"sampling": specification["sampling"]} if specification["format_version"] == 4 else {}),
            **{key: specification[key] for key in
            ("target", "build_mode", "collector", "host", "environment", "policy", "method_review")},
            "baseline": specification["binaries"]["baseline"], "manifest": evidence["manifest"],
            "envelopes": collector.derive_envelopes(evidence)}


def persist_v2_smoke(root, build_mode="normal"):
    root.mkdir()
    (root / "started").mkdir()
    (root / "attempts").mkdir()
    specification = v2_plan(root.parent, "smoke", build_mode)
    specification["workloads"] = ["single-small96x64"]
    specification["schedule"] = collector.schedule("smoke", specification["workloads"], 2)
    specification["execution_limits"] = collector.execution_limits("smoke", len(specification["schedule"]))
    specification["method_review"] = None
    collector.save(root / "plan.json", specification)
    for index, task in enumerate(specification["schedule"]):
        binary = specification["binaries"][task["role"]]
        attempt = {"task": task, "argv": collector.command(binary, task, "smoke", 2),
                   "plan_sha256": collector.sha256(root / "plan.json"), "started_unix_ns": index + 1,
                   "campaign_elapsed_before": index, "output_limit_bytes": 8 * collector.MIB,
                   "cleanup_seconds": 5, "runtime_binding_seconds": 0}
        collector.save(root / "started" / f"{task['id']}.json", attempt)
        observed = v2_manifest(build_mode) if task["workload"] is None else v2_payload(
            task["workload"], build_mode, smoke=True)
        attempt.update(identity_before=binary, identity_after=binary, identity_error=None,
                       campaign_elapsed_after=index + 0.1, record=process_record(observed), timeout_seconds=90)
        collector.save(root / "attempts" / f"{task['id']}.json", attempt)
    return specification


class ProtocolTwoTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def arguments(self, mode="smoke", build_mode="normal"):
        argv = [mode, "--protocol-version", "2", "--build-mode", build_mode, "--target", TARGET,
                "--baseline", "baseline", "--baseline-metadata", "build.json", "--output", str(self.root / mode)]
        if mode == "smoke":
            argv += ["--workload", "single-small96x64"]
        else:
            argv += ["--exclusive-host"]
        return collector.parser().parse_args(argv)

    @contextlib.contextmanager
    def smoke_context(self, args, records, clock=None, binding=None):
        original_runtime = collector.loaded_runtime
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(collector, "host_identity", return_value={"target": TARGET}))
            stack.enter_context(mock.patch.object(collector, "binary_identity",
                                                  return_value=binding or v2_identity(self.root, args.build_mode)))
            stack.enter_context(mock.patch.object(collector, "check_files"))
            stack.enter_context(mock.patch.object(collector, "exclusive_collector",
                                                  return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(
                collector, "loaded_runtime",
                side_effect=lambda value, identity=None, **kwargs: original_runtime(value, identity)))
            if clock is not None:
                stack.enter_context(mock.patch.object(collector.time, "monotonic", side_effect=clock))
            launch = stack.enter_context(mock.patch.object(collector, "run_process", side_effect=records))
            yield launch

    def launch_smoke(self, args, records, clock=None, binding=None):
        with self.smoke_context(args, records, clock, binding) as launch:
            result = collector.collect(args, "smoke")
        return result, launch

    @contextlib.contextmanager
    def replace_document_after_read(self, target, replacement):
        original_bytes, original_text = Path.read_bytes, Path.read_text

        def read_bytes(path, *args, **kwargs):
            content = original_bytes(path, *args, **kwargs)
            if path == target:
                path.write_bytes(replacement)
            return content

        def read_text(path, *args, **kwargs):
            content = original_text(path, *args, **kwargs)
            if path == target:
                path.write_bytes(replacement)
            return content

        with mock.patch.object(Path, "read_bytes", read_bytes), \
                mock.patch.object(Path, "read_text", read_text):
            yield

    def test_plan_replacement_cannot_bind_attempts_to_unparsed_bytes(self):
        directory = self.root / "smoke"
        specification = persist_v2_smoke(directory)
        path = directory / "plan.json"
        original = path.read_bytes()
        specification["environment"]["PLAN_SNAPSHOT_TEST"] = "replacement"
        replacement = collector.document(specification)
        replacement_hash = hashlib.sha256(replacement).hexdigest()
        for folder in ("started", "attempts"):
            for receipt in (directory / folder).glob("*.json"):
                value = collector.load(receipt)
                value["plan_sha256"] = replacement_hash
                receipt.write_bytes(collector.document(value))
        with self.replace_document_after_read(path, replacement):
            result = collector.report(directory)
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["performance_qualified"])
        self.assertEqual(result["plan_sha256"], hashlib.sha256(original).hexdigest())
        self.assertEqual(sum(issue.get("category") == "protocol" for issue in result["issues"]), 2)
        self.assertEqual(path.read_bytes(), replacement)

    def test_failed_started_receipt_hash_names_the_bytes_actually_validated(self):
        directory = self.root / "smoke"
        persist_v2_smoke(directory)
        path = directory / "started/0001.json"
        replacement = path.read_bytes()
        invalid = collector.strict_json(replacement.decode("utf-8"))
        invalid["started_unix_ns"] += 1
        original = collector.document(invalid)
        path.write_bytes(original)
        with self.replace_document_after_read(path, replacement):
            result = collector.report(directory)
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["performance_qualified"])
        self.assertTrue(any(issue.get("category") == "protocol"
                            and issue.get("file") == "attempts/0001.json" for issue in result["issues"]))
        retained = next(item for item in result["attempts"] if item["file"] == "0001.json")
        self.assertEqual(retained["started_sha256"], hashlib.sha256(original).hexdigest())
        self.assertEqual(path.read_bytes(), replacement)

    def test_damaged_started_receipt_retains_hash_and_other_attempts(self):
        for damage, raw in (("truncated", b'{"task":'), ("encoding", b"\xff"), ("non-object", b"[]")):
            with self.subTest(damage=damage):
                directory = self.root / damage
                persist_v2_smoke(directory)
                path = directory / "started/0001.json"
                path.write_bytes(raw)
                result = collector.report(directory)
                self.assertEqual(result["status"], "failed")
                self.assertFalse(result["performance_qualified"])
                self.assertEqual({item["file"] for item in result["attempts"]}, {"0000.json", "0001.json"})
                retained = next(item for item in result["attempts"] if item["file"] == "0001.json")
                self.assertEqual(retained["started_sha256"], hashlib.sha256(raw).hexdigest())
                self.assertTrue(any(issue.get("category") == "protocol"
                                    and issue.get("file") == "attempts/0001.json" for issue in result["issues"]))

    def test_created_plan_replacement_cannot_relabel_executed_environment(self):
        args = self.arguments()
        path = Path(args.output) / "plan.json"
        original_open = Path.open
        written = []

        @contextlib.contextmanager
        def replace_created(stream):
            with stream:
                yield stream
            original = path.read_bytes()
            written.append(original)
            replacement = collector.strict_json(original.decode("utf-8"))
            replacement["environment"]["PLAN_SNAPSHOT_TEST"] = "not-executed"
            path.write_bytes(collector.document(replacement))

        def open_path(current, mode="r", *args, **kwargs):
            stream = original_open(current, mode, *args, **kwargs)
            return replace_created(stream) if current == path and mode == "xb" else stream

        records = [process_record(v2_manifest()),
                   process_record(v2_payload("single-small96x64", smoke=True))]
        with mock.patch.object(Path, "open", open_path):
            result, launch = self.launch_smoke(args, records)
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["performance_qualified"])
        self.assertEqual(len(written), 1)
        written_hash = hashlib.sha256(written[0]).hexdigest()
        self.assertNotEqual(written_hash, collector.sha256(path))
        for call in launch.call_args_list:
            self.assertNotIn("PLAN_SNAPSHOT_TEST", call.kwargs["env"])
        for receipt in (Path(args.output) / "attempts").glob("*.json"):
            self.assertEqual(collector.load(receipt)["plan_sha256"], written_hash)

    def test_non_object_plan_is_a_validation_failure(self):
        directory = self.root / "invalid-plan"
        directory.mkdir()
        (directory / "plan.json").write_bytes(b"[]")
        with self.assertRaises(ValueError):
            collector.evaluate(directory)

    def test_capable_binary_keeps_ordinary_smoke_off_and_does_not_accept_claimed_enablement(self):
        binding = timing_identity(v2_identity(self.root))
        declaration = collector.strict_json(binding["build"]["provenance_commands"][0]["record"]["stdout"])
        for enabled in (False, True):
            with self.subTest(enabled=enabled):
                args = self.arguments()
                args.output = str(self.root / f"capable-{enabled}")
                value = v2_payload("single-small96x64", smoke=True)
                value.update(timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=enabled)
                result, launch = self.launch_smoke(
                    args, [process_record(declaration), process_record(value)], binding=binding)
                self.assertEqual(result["status"], "failed" if enabled else "passed")
                self.assertFalse(result["performance_qualified"])
                self.assertTrue(all("--timing-boundaries" not in call.args[0] for call in launch.call_args_list))

    def test_exact_source_rows_and_normal_null_metrics(self):
        for build_mode, count in (("normal", 18), ("instrumented", 21)):
            with self.subTest(build_mode=build_mode):
                frozen = collector.manifest_identity(v2_manifest(build_mode), TARGET, "4.12.0", 2, build_mode)
                self.assertEqual(len(frozen["workloads"]), count)
                self.assertIn("selection-k16-dense1080p-keepall", [row["id"] for row in frozen["workloads"]])
                self.assertNotIn("selection-k16-dense1080p-keep19", [row["id"] for row in frozen["workloads"]])
                summary = collector.observations(v2_payload("single-small96x64", build_mode), frozen,
                                                 "single-small96x64", False)
                self.assertEqual(summary["unavailable"], [])
                self.assertEqual(summary["peak_heap_bytes"] is None, build_mode == "normal")
                if build_mode == "normal":
                    self.assertIsNone(summary["heap_growth_bytes"])
                    self.assertIsNone(summary["heap_residual_bytes"])

    def test_each_unobserved_normal_metric_rejects_zero_or_missing(self):
        frozen = collector.manifest_identity(v2_manifest(), TARGET, "4.12.0", 2, "normal")
        for key in collector.NORMAL_UNAVAILABLE_METRICS:
            for missing in (False, True):
                with self.subTest(key=key, missing=missing):
                    value = v2_payload("single-small96x64")
                    if missing:
                        value["workloads"][0]["metrics"].pop(key)
                    else:
                        value["workloads"][0]["metrics"][key] = 0
                    with self.assertRaisesRegex(ValueError, "explicit null|metric key set"):
                        collector.observations(value, frozen, "single-small96x64", False)

    def test_unavailable_declaration_cannot_hide_another_row_or_metric(self):
        for key in ("metrics", "workloads"):
            value = v2_manifest()
            value["unavailable"][key].append("invented")
            with self.assertRaisesRegex(ValueError, "unavailable declaration"):
                collector.manifest_identity(value, TARGET, "4.12.0", 2, "normal")
        value = v2_manifest()
        value["workloads"].append(manifest()["workloads"][-1])
        with self.assertRaisesRegex(ValueError, "incomplete workload"):
            collector.manifest_identity(value, TARGET, "4.12.0", 2, "normal")

    def test_normal_cpu_or_rss_missing_stays_unavailable_not_zero(self):
        frozen = collector.manifest_identity(v2_manifest(), TARGET, "4.12.0", 2, "normal")
        for missing in ("cpu", "rss"):
            value = v2_payload("single-small96x64")
            metrics = value["workloads"][0]["metrics"]
            if missing == "cpu":
                value["workloads"][0]["samples"][0]["cpu_ns"] = None
                metrics.update(cpu_ns=None, cpu_reason="CPU unavailable")
            else:
                metrics.update(peak_rss_bytes=None, native_resource_reason="resident API unavailable")
            summary = collector.observations(value, frozen, "single-small96x64", False)
            self.assertIsNone(summary["cpu_ns" if missing == "cpu" else "peak_rss_bytes"])
            self.assertTrue(summary["unavailable"])

    def test_manifest_and_observation_compiled_mode_must_match_requested_mode(self):
        with self.assertRaisesRegex(ValueError, "compiled build mode"):
            collector.manifest_identity(v2_manifest("instrumented"), TARGET, "4.12.0", 2, "normal")
        frozen = collector.manifest_identity(v2_manifest(), TARGET, "4.12.0", 2, "normal")
        with self.assertRaisesRegex(ValueError, "compiled build mode"):
            collector.observations(v2_payload("single-small96x64", "instrumented"), frozen, "single-small96x64", False)

    def test_source_fixture_drift_cannot_reuse_matching_top_level_digest(self):
        frozen = collector.manifest_identity(v2_manifest(), TARGET, "4.12.0", 2, "normal")
        value = v2_payload("single-small96x64")
        value["fixture"]["direct_formats"]["helper_source_sha256"] = "9" * 64
        with self.assertRaisesRegex(ValueError, "source/fixture"):
            collector.observations(value, frozen, "single-small96x64", False)

    def test_attested_source_probe_and_mode_drift_reject_before_launch(self):
        for field, replacement in (("source_tree", "9" * 40), ("build_mode", "instrumented")):
            with self.subTest(field=field):
                binding = v2_identity(self.root)
                binding["build"][field] = replacement
                args = self.arguments()
                with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
                      mock.patch.object(collector, "binary_identity", return_value=binding),
                      mock.patch.object(collector, "run_process") as launch,
                      self.assertRaisesRegex(ValueError, "source probe|mode/protocol")):
                    collector.collect(args, "smoke")
                launch.assert_not_called()
                self.assertFalse(Path(args.output).exists())

    def test_endpoint_metadata_cannot_be_promoted_by_self_declared_clearance(self):
        binding = v2_identity(self.root)
        binding["build"].update(provenance_complete=True, runtime_closure={"before": True, "after": True,
                                                                          "measurement_time": True})
        with self.assertRaises(ValueError):
            collector.runtime_closure_admission(binding, TARGET, "normal")

    def test_unimplemented_observer_method_is_not_clearance(self):
        binding = v2_identity(self.root)
        probe = binding["build"]["provenance_commands"][0]["record"]
        value = collector.strict_json(probe["stdout"])
        value["loaded_native_runtime"]["method"] = "invented-complete-loader-observer"
        probe["stdout"] = json.dumps(value)
        with self.assertRaisesRegex(ValueError, "evidence method"):
            collector.runtime_closure_admission(binding, TARGET, "normal")

    def test_v2_smoke_runs_both_modes_without_qualification(self):
        for build_mode in ("normal", "instrumented"):
            with self.subTest(build_mode=build_mode):
                args = self.arguments(build_mode=build_mode)
                args.output = str(self.root / build_mode)
                result, launch = self.launch_smoke(args, [
                    process_record(v2_manifest(build_mode)),
                    process_record(v2_payload("single-small96x64", build_mode, smoke=True))])
                self.assertEqual(result["status"], "passed")
                self.assertFalse(result["performance_qualified"])
                self.assertEqual(result["actual_consumer_coverage"], "not-implemented")
                self.assertEqual(launch.call_count, 2)
                self.assertEqual(launch.call_args_list[0].args[0][1:], ["--protocol-version", "2", "--list"])
                self.assertEqual(len(result["attempts"]), 2)
                self.assertTrue(all("started_sha256" in item for item in result["attempts"]))

    def test_mode_mismatch_stops_future_slots_and_retains_raw_attempt(self):
        args = self.arguments()
        wrong = process_record(v2_manifest("instrumented"))
        wrong["stderr"] = "retained mode diagnostic"
        result, launch = self.launch_smoke(args, [wrong])
        self.assertEqual(launch.call_count, 1)
        self.assertEqual(result["status"], "failed")
        self.assertTrue(any(issue["status"] == "not-run" for issue in result["issues"]))
        attempt = collector.load(Path(args.output) / "attempts/0000.json")
        self.assertEqual(attempt["record"], wrong)
        self.assertIsNone(attempt["identity_error"])
        self.assertEqual(attempt["identity_before"], attempt["identity_after"])
        self.assertIn("semantic", {issue.get("category") for issue in result["issues"]})

    def test_oracle_failure_keeps_identity_and_runs_later_rows(self):
        args = self.arguments()
        args.workload = None
        rows = list(collector.definitions_for("normal"))
        records = [process_record(v2_manifest())]
        for workload in rows:
            value = v2_payload(workload, smoke=True)
            if workload == "single-small96x64":
                value["workloads"][0]["oracle_failures"] = 1
                value["workloads"][0]["samples"][0]["correct"] = False
            records.append(process_record(value))
        result, launch = self.launch_smoke(args, records)
        self.assertEqual(launch.call_count, len(rows) + 1)
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["performance_qualified"])
        self.assertIn("semantic", {issue.get("category") for issue in result["issues"]})
        self.assertNotIn("identity", {issue.get("category") for issue in result["issues"]})
        self.assertFalse(any(issue["status"] == "not-run" for issue in result["issues"]))
        failed = collector.load(Path(args.output) / "attempts" / f"{rows.index('single-small96x64') + 1:04}.json")
        self.assertIsNone(failed["identity_error"])
        self.assertEqual(failed["identity_before"], failed["identity_after"])

    def test_campaign_deadline_is_independent_of_child_limit(self):
        args = self.arguments()
        result, launch = self.launch_smoke(args, [process_record(v2_manifest())], clock=[0, 0, 0, 1, 901])
        self.assertEqual(launch.call_count, 1)
        self.assertEqual(launch.call_args.kwargs["timeout_seconds"], 90)
        self.assertEqual(result["status"], "incomplete")
        self.assertTrue(any(issue["status"] == "not-run" for issue in result["issues"]))
        retained = collector.load(Path(args.output) / "plan.json")
        self.assertEqual(retained["execution_limits"], collector.execution_limits("smoke", 2))

    def test_incremental_pause_consumes_original_campaign_deadline(self):
        args = self.arguments()
        with self.smoke_context(
                args, [process_record(v2_manifest())], clock=[0, 0, 0, 1, 901]):
            with contextlib.closing(collector.collection_steps(args, "smoke")) as steps:
                checkpoint = next(steps)
                retained = Path(checkpoint["path"]).read_bytes()
                with self.assertRaises(StopIteration) as finished:
                    next(steps)
            result = finished.exception.value
            self.assertEqual(result["status"], "incomplete")
            self.assertEqual([item["file"] for item in result["attempts"]], ["0000.json"])
            self.assertEqual([item["attempt"] for item in result["issues"]],
                             ["0001:baseline:single-small96x64"])
            self.assertEqual(Path(checkpoint["path"]).read_bytes(), retained)
            self.assertFalse((Path(args.output) / "started/0001.json").exists())

    def test_incremental_close_preserves_prefix_and_releases_collector(self):
        args = self.arguments()
        original_lock = collector.exclusive_collector
        records = [process_record(v2_manifest()), process_record(v2_manifest()),
                   process_record(v2_payload("single-small96x64", smoke=True))]
        with (self.smoke_context(args, records),
              mock.patch.object(collector, "exclusive_collector", original_lock),
              mock.patch.object(collector.tempfile, "gettempdir", return_value=str(self.root))):
            with contextlib.closing(collector.collection_steps(args, "smoke")) as steps:
                checkpoint = next(steps)
                retained = Path(checkpoint["path"]).read_bytes()
                with self.assertRaisesRegex(ValueError, "collector lock exists"):
                    collector.collect(args, "smoke")
            partial = collector.report(args.output)
            self.assertEqual(partial["status"], "incomplete")
            self.assertFalse(partial["performance_qualified"])
            self.assertEqual([item["file"] for item in partial["attempts"]], ["0000.json"])
            with self.assertRaises(FileExistsError):
                collector.collect(args, "smoke")
            args.output = str(self.root / "independent-smoke")
            completed = collector.collect(args, "smoke")
            self.assertEqual(completed["status"], "passed")
            self.assertEqual([item["file"] for item in completed["attempts"]],
                             ["0000.json", "0001.json"])
            self.assertEqual(Path(checkpoint["path"]).read_bytes(), retained)

    def test_incremental_collection_freezes_supplied_arguments(self):
        args = self.arguments()
        records = [process_record(v2_manifest()),
                   process_record(v2_payload("single-small96x64", smoke=True))]
        with self.smoke_context(args, records):
            with contextlib.closing(collector.collection_steps(args, "smoke")) as steps:
                first = next(steps)
                args.target = collector.TARGETS[1]
                args.build_mode = "instrumented"
                second = next(steps)
                with self.assertRaises(StopIteration) as finished:
                    next(steps)
            result = finished.exception.value
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["target"], TARGET)
            self.assertEqual(result["build_mode"], "normal")
            self.assertEqual(second["campaign_deadline_monotonic"],
                             first["campaign_deadline_monotonic"])
            self.assertEqual([item["file"] for item in result["attempts"]],
                             ["0000.json", "0001.json"])

    def test_incremental_cleanup_failure_cannot_launch_after_checkpoint(self):
        args = self.arguments()
        failed = process_record(v2_manifest())
        failed.update(cleanup_ok=False, stderr="owned cleanup incomplete")
        with self.smoke_context(args, [failed]):
            with contextlib.closing(collector.collection_steps(args, "smoke")) as steps:
                checkpoint = next(steps)
                with self.assertRaises(StopIteration) as finished:
                    next(steps)
            result = finished.exception.value
            self.assertEqual(result["status"], "failed")
            self.assertIn("cleanup", {item.get("category") for item in result["issues"]})
            self.assertFalse((Path(args.output) / "started/0001.json").exists())
            self.assertEqual(collector.load(checkpoint["path"])["record"], failed)

    def test_timeout_output_or_cleanup_failure_preserves_partial_streams_and_slots(self):
        for flag in ("timed_out", "output_limited", "cleanup_ok"):
            with self.subTest(flag=flag):
                args = self.arguments()
                args.output = str(self.root / flag)
                failed = process_record("partial JSON")
                failed.update(stderr="partial diagnostics", exit_code=1)
                failed[flag] = flag != "cleanup_ok"
                result, launch = self.launch_smoke(args, [failed])
                self.assertEqual(launch.call_count, 1)
                self.assertEqual(result["status"], "failed")
                self.assertTrue(any(issue["status"] == "not-run" for issue in result["issues"]))
                self.assertEqual(collector.load(Path(args.output) / "attempts/0000.json")["record"], failed)

    def test_retained_v2_bounds_and_started_receipt_are_revalidated(self):
        directory = self.root / "retained"
        persist_v2_smoke(directory)
        path = directory / "attempts/0001.json"
        original = path.read_bytes()
        for field, value in (("timeout_seconds", 91), ("output_limit_bytes", 9 * collector.MIB),
                             ("cleanup_seconds", 6), ("campaign_elapsed_after", 901),
                             ("started_unix_ns", 999)):
            with self.subTest(field=field):
                attempt = collector.strict_json(original.decode())
                attempt[field] = value
                path.write_bytes(collector.document(attempt))
                result = collector.report(directory)
                self.assertEqual(result["status"], "failed")
                self.assertFalse(result["performance_qualified"])
        path.write_bytes(original)
        self.assertEqual(collector.report(directory)["status"], "passed")
        collector.save(directory / "attempts/9999.json", {})
        with self.assertRaisesRegex(ValueError, "hidden retry"):
            collector.report(directory)

    def test_started_only_is_incomplete_and_immutable_creation_rejects_replacement(self):
        directory = self.root / "started-only"
        persist_v2_smoke(directory)
        (directory / "attempts/0001.json").unlink()
        result = collector.report(directory)
        self.assertEqual(result["status"], "incomplete")
        self.assertTrue(any(issue["status"] == "incomplete" for issue in result["issues"]))
        self.assertTrue(result["attempts"][-1]["file"].startswith("started/"))
        before = (directory / "started/0001.json").read_bytes()
        with self.assertRaises(FileExistsError):
            collector.save(directory / "started/0001.json", {"replacement": True})
        self.assertEqual((directory / "started/0001.json").read_bytes(), before)


    def test_attestation_retains_mode_manifest_and_endpoint_scope(self):
        for build_mode in ("normal", "instrumented"):
            with self.subTest(build_mode=build_mode):
                binary = self.root / f"{build_mode}-binary"
                native = self.root / "libopencv_core.dylib"
                metadata = self.root / f"{build_mode}-build.json"
                binary.write_bytes(b"explicit executable fixture")
                native.write_bytes(b"native fixture bytes")
                (self.root / "Cargo.lock").write_bytes(b"fixture lockfile")
                observed = v2_manifest(build_mode)
                observed["target"] = NATIVE_TARGET
                observed["loaded_native_runtime"] = {
                    "method": ("K32EnumProcessModules-GetModuleFileNameW" if NATIVE_TARGET == collector.TARGETS[1]
                               else "dyld-loaded-image-paths"),
                    "images": [{"path": str(native), "sha256": collector.sha256(native)}]}
                records = [process_record(observed), process_record("a" * 40 + "\n"),
                           process_record("c" * 40 + "\n"), process_record(""),
                           process_record("rustc fixture\n")]
                args = collector.parser().parse_args([
                    "attest", "--protocol-version", "2", "--build-mode", build_mode, "--target", NATIVE_TARGET,
                    "--binary", str(binary), "--build-argv-json", '["cargo","bench","--locked","--no-run"]',
                    "--output", str(metadata)])
                with (mock.patch.object(collector, "ROOT", self.root),
                      mock.patch.object(collector, "host_identity", return_value={"target": NATIVE_TARGET}),
                      mock.patch.object(collector.shutil, "which", return_value=sys.executable),
                      mock.patch.object(collector, "run_process", side_effect=records) as launch):
                    collector.attest(args)
                build = collector.load(metadata)
                self.assertEqual(build["build_mode"], build_mode)
                self.assertEqual(build["benchmark_manifest"]["build_mode"], build_mode)
                self.assertEqual(launch.call_args_list[0].args[0], [str(binary), "--protocol-version", "2", "--list"])
                self.assertEqual(collector.load(str(metadata) + ".probe.json")["commands"], build["provenance_commands"])
                binding = {"path": str(binary), "build": build}
                with self.assertRaisesRegex(ValueError, "missing runtime closure"):
                    collector.runtime_closure_admission(binding, NATIVE_TARGET, build_mode)

    def test_attestation_manifest_mismatch_stops_remaining_probes(self):
        binary = self.root / "binary"
        binary.write_bytes(b"explicit executable fixture")
        for mismatch in ("build_mode", "target"):
            with self.subTest(mismatch=mismatch):
                metadata = self.root / f"{mismatch}-build.json"
                observed = v2_manifest("instrumented" if mismatch == "build_mode" else "normal")
                if mismatch == "target":
                    observed["target"] = collector.TARGETS[1]
                first = process_record(observed)
                args = collector.parser().parse_args([
                    "attest", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
                    "--binary", str(binary), "--build-argv-json", '["cargo","bench","--no-run"]',
                    "--output", str(metadata)])
                with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
                      mock.patch.object(collector.shutil, "which", return_value=sys.executable),
                      mock.patch.object(collector, "run_process", return_value=first) as launch,
                      self.assertRaises(ValueError)):
                    collector.attest(args)
                self.assertEqual(launch.call_count, 1)
                probe = collector.load(str(metadata) + ".probe.json")
                self.assertEqual(len(probe["commands"]), 1)
                self.assertEqual(len(probe["schedule"]), 5)
                self.assertEqual(probe["commands"][0]["record"], first)
                self.assertFalse(metadata.exists())

    def test_attestation_campaign_deadline_preserves_probe_and_future_slots(self):
        binary = self.root / "binary"
        binary.write_bytes(b"explicit executable fixture")
        metadata = self.root / "deadline-build.json"
        args = collector.parser().parse_args([
            "attest", "--protocol-version", "2", "--build-mode", "normal", "--target", NATIVE_TARGET,
            "--binary", str(binary), "--build-argv-json", '["cargo","bench","--no-run"]',
            "--output", str(metadata)])
        native = self.root / "libopencv_core.dylib"
        native.write_bytes(b"native fixture bytes")
        observed = v2_manifest()
        observed["target"] = NATIVE_TARGET
        observed["loaded_native_runtime"] = {
            "method": ("K32EnumProcessModules-GetModuleFileNameW" if NATIVE_TARGET == collector.TARGETS[1]
                       else "dyld-loaded-image-paths"),
            "images": [{"path": str(native), "sha256": collector.sha256(native)}]}
        first = process_record(observed)
        with (mock.patch.object(collector, "host_identity", return_value={"target": NATIVE_TARGET}),
              mock.patch.object(collector.shutil, "which", return_value=sys.executable),
              mock.patch.object(collector.time, "monotonic", side_effect=[0, 0, 901]),
              mock.patch.object(collector, "run_process", return_value=first) as launch,
              self.assertRaisesRegex(ValueError, "campaign deadline")):
            collector.attest(args)
        self.assertEqual(launch.call_count, 1)
        self.assertFalse(metadata.exists())
        probe = collector.load(str(metadata) + ".probe.json")
        self.assertEqual(len(probe["schedule"]), 5)
        self.assertEqual(probe["execution_limits"], collector.execution_limits("smoke", 5))
        self.assertEqual(probe["commands"][0]["record"], first)

    def test_v2_unavailable_workload_or_missing_build_mode_never_launches(self):
        for change in ("workload", "build_mode"):
            args = self.arguments()
            if change == "workload":
                args.workload = "watch-exact-rgba"
            else:
                args.build_mode = None
            with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
                  mock.patch.object(collector, "binary_identity", return_value=v2_identity(self.root)),
                  mock.patch.object(collector, "run_process") as launch,
                  self.assertRaisesRegex(ValueError, "unavailable|requires --build-mode")):
                collector.collect(args, "smoke")
            launch.assert_not_called()

    def test_current_smoke_cli_keeps_unavailable_accounting_descriptive(self):
        args = self.arguments()
        args.protocol_version = 4
        value = v4_payload(warmups=1, count=3)
        value["cpu_timer_resolution_ns"] = None
        row = value["workloads"][0]
        for sample in row["warmup_samples"] + row["samples"]:
            sample["cpu_ns"] = None
        row["metrics"].update(cpu_ns=None, cpu_reason="optional accounting unavailable")
        row["retained_loop_cpu"].update(start_ns=None, end_ns=None, cpu_ns=None,
                                        reason="optional accounting unavailable")
        argv = ["smoke", "--protocol-version", "4", "--build-mode", "normal", "--target", TARGET,
                "--baseline", "baseline", "--baseline-metadata", "build.json", "--output", args.output,
                "--workload", "single-small96x64"]
        with self.smoke_context(args, [process_record(v4_manifest()), process_record(value)],
                                binding=v4_identity(self.root)) as launch, \
                mock.patch.object(collector, "reviewed_method", side_effect=AssertionError("no method admission")), \
                mock.patch.object(collector, "runtime_closure_admission", side_effect=AssertionError("no qualification closure")):
            stream = io.StringIO()
            with contextlib.redirect_stdout(stream):
                code = collector.main(argv)
        self.assertEqual((code, launch.call_count), (0, 2))
        result = json.loads(stream.getvalue())
        self.assertEqual((result["report_format"], result["execution_status"], result["qualification_status"]),
                         ("component-measurement-report-v1", "completed", "not-requested"))
        self.assertFalse(result["performance_qualified"])
        self.assertEqual(result["issues"], [])
        observation = result["process_observations"]["baseline"]["single-small96x64"][0]
        self.assertIsNone(observation["cpu_ns"])
        self.assertIsNone(observation["retained_loop_cpu_ns"])
        self.assertIsNone(observation["peak_heap_bytes"])
        self.assertTrue(observation["unavailable"])
        self.assertTrue(all(call.kwargs["timeout_seconds"] <= 90
                            and call.kwargs["cleanup_seconds"] == 5 for call in launch.call_args_list))
        retained = {path: path.read_bytes() for path in Path(args.output).rglob("*.json")}
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(collector.main(["report", args.output]), 0)
        self.assertEqual(retained, {path: path.read_bytes() for path in retained})

    def test_numerical_control_report_does_not_admit_claim_or_block_separate_smoke(self):
        calibration = v2_parsed_evidence(self.root, "calibration")
        control = v2_parsed_evidence(self.root, "control")
        control["plan"]["report_format"] = collector.REPORT_FORMAT
        for row in control["observations"]["control"]["single-small96x64"]:
            row["p95_ns"] *= 2
        original = copy.deepcopy(control)
        with mock.patch.object(collector, "evaluate", side_effect=[control, calibration]), \
                contextlib.redirect_stdout(io.StringIO()) as output:
            code = collector.main(["report", str(self.root / "control")])
        self.assertEqual(code, 1)
        failed = json.loads(output.getvalue())
        self.assertEqual((failed["status"], failed["execution_status"], failed["qualification_status"]),
                         ("failed", "completed", "failed"))
        self.assertFalse(failed["performance_qualified"])
        self.assertEqual(control, original)
        args = self.arguments()
        lock = collector.exclusive_collector
        with self.smoke_context(args, [process_record(v2_manifest()),
                                      process_record(v2_payload("single-small96x64", smoke=True))]) as launch, \
                mock.patch.object(collector, "exclusive_collector", lock), \
                mock.patch.object(collector.tempfile, "gettempdir", return_value=str(self.root)):
            result = collector.collect(args, "smoke")
        self.assertEqual((launch.call_count, result["status"]), (2, "passed"))
        self.assertFalse((self.root / "mado-template-matching-efficiency.lock").exists())
        self.assertFalse(result["performance_qualified"])
        self.assertEqual(control, original)

    def test_cleanup_identity_and_unknown_launch_keep_lock_across_new_output(self):
        lock = collector.exclusive_collector
        for failure in ("cleanup", "unknown-cleanup", "identity", "unknown-launch"):
            with self.subTest(failure=failure):
                lock_root = self.root / failure
                lock_root.mkdir()
                args = self.arguments()
                args.output = str(lock_root / "first")
                record = process_record(v2_manifest())
                if failure in ("cleanup", "unknown-cleanup"):
                    record["cleanup_ok"] = False if failure == "cleanup" else None
                records = [KeyboardInterrupt("unknown launch")] if failure == "unknown-launch" else [record]
                with self.smoke_context(args, records) as launch, \
                        mock.patch.object(collector, "exclusive_collector", lock), \
                        mock.patch.object(collector.tempfile, "gettempdir", return_value=str(lock_root)), \
                        mock.patch.object(collector, "check_files", side_effect=(
                            [None, ValueError("binary changed")] if failure == "identity" else None)):
                    if failure == "unknown-launch":
                        with self.assertRaises(KeyboardInterrupt):
                            collector.collect(args, "smoke")
                    else:
                        result = collector.collect(args, "smoke")
                        self.assertEqual(result["status"], "failed")
                    self.assertEqual(launch.call_count, 1)
                    path = lock_root / "mado-template-matching-efficiency.lock"
                    retained = path.read_bytes()
                    owner = json.loads(retained)
                    self.assertEqual(owner["pid"], os.getpid())
                    self.assertEqual(owner["evidence"], str(lock_root / "first"))
                    args.output = str(lock_root / "second")
                    with self.assertRaises(ValueError):
                        collector.collect(args, "smoke")
                    self.assertEqual(launch.call_count, 1)
                    self.assertEqual(path.read_bytes(), retained)
                    self.assertFalse(Path(args.output).exists())

    def test_safe_prelaunch_failure_releases_lock_without_claiming_execution(self):
        lock = collector.exclusive_collector
        args = self.arguments()
        Path(args.output).mkdir()
        with self.smoke_context(args, []) as launch, \
                mock.patch.object(collector, "exclusive_collector", lock), \
                mock.patch.object(collector.tempfile, "gettempdir", return_value=str(self.root)):
            with self.assertRaises(FileExistsError):
                collector.collect(args, "smoke")
        launch.assert_not_called()
        self.assertFalse((self.root / "mado-template-matching-efficiency.lock").exists())

    def test_ordinary_semantic_failure_is_non_success_but_clean_owner_releases_lock(self):
        args = self.arguments()
        bad = v2_payload("single-small96x64", smoke=True)
        bad["workloads"][0]["samples"][0]["correct"] = False
        lock = collector.exclusive_collector
        records = [process_record(v2_manifest()), process_record(bad),
                   process_record(v2_manifest()), process_record(v2_payload("single-small96x64", smoke=True))]
        with self.smoke_context(args, records) as launch, \
                mock.patch.object(collector, "exclusive_collector", lock), \
                mock.patch.object(collector.tempfile, "gettempdir", return_value=str(self.root)):
            failed = collector.collect(args, "smoke")
            self.assertEqual((failed["status"], failed["execution_status"]), ("failed", "failed"))
            self.assertFalse((self.root / "mado-template-matching-efficiency.lock").exists())
            args.output = str(self.root / "next-diagnostic")
            completed = collector.collect(args, "smoke")
        self.assertEqual((launch.call_count, completed["status"]), (4, "passed"))


def save_method_review(root, specification, campaign_seconds=None):
    version = specification["format_version"]
    review = {"format_version": version, "kind": "component-method-review", "decision": "accepted",
              **{key: specification[key] for key in
                 ("target", "build_mode", "host", "environment", "collector", "policy", "workloads")},
              "baseline": specification["binaries"]["baseline"],
              "candidate": specification["binaries"].get("candidate")
                           or (v4_identity if version == 4 else v3_identity if version == 3 else v2_identity)(
                               root, specification["build_mode"], "candidate"),
              "reviewer": "fixture reviewer", "rationale": "deterministic apparatus decision only",
              "source_host_cost_review": "synthetic finite component plan; no runtime clearance",
              "runtime_observer_eligibility_review": "synthetic receipt validation only; no native eligibility claim",
              "stage_limits": {mode: collector.execution_limits(
                  mode, len(collector.schedule(mode, specification["workloads"], version)))
                  for mode in ("calibration", "control", "comparison")}}
    if campaign_seconds is not None:
        for mode, seconds in campaign_seconds.items():
            review["stage_limits"][mode]["campaign_timeout_seconds"] = seconds
    if "runtime_observer_policy" in specification:
        review["runtime_observer_policy"] = specification["runtime_observer_policy"]
    if version == 4:
        review["sampling"] = specification["sampling"]
    path = root / "method.json"
    collector.save(path, review)
    return {"path": str(path), "sha256": collector.sha256(path),
            **({"stage_limits": review["stage_limits"]}
               if version >= 3 and len(specification["workloads"]) == 1 else {})}


class ProtocolTwoAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.calibration = v2_parsed_evidence(self.root, "calibration")
        self.calibration["plan"]["runtime_observer_policy"] = "macos-dyld-observer-on-warm-v1"
        self.identity = timing_identity(v2_identity(self.root))
        self.calibration["plan"]["binaries"]["baseline"] = self.identity
        self.binding = save_method_review(self.root, self.calibration["plan"])
        self.calibration["plan"]["method_review"] = self.binding

    def calibration_args(self):
        return collector.parser().parse_args([
            "calibrate", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--baseline", "baseline", "--baseline-metadata", "build.json", "--exclusive-host",
            "--method-review", self.binding["path"], "--method-review-sha256", self.binding["sha256"],
            "--output", str(self.root / "calibration")])

    def test_bound_method_does_not_authorize_endpoint_only_calibration(self):
        args = self.calibration_args()
        with (mock.patch.object(collector, "host_identity", return_value=self.calibration["plan"]["host"]),
              mock.patch.object(collector, "environment", return_value={}),
              mock.patch.object(collector, "binary_identity", return_value=self.identity),
              mock.patch.object(collector, "run_process") as launch,
              self.assertRaisesRegex(ValueError, "missing runtime closure.*zero launches")):
            collector.collect(args, "calibration")
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_review_cannot_authorize_an_incapable_binary(self):
        binding = v2_identity(self.root)
        review = collector.load(self.binding["path"])
        review["baseline"] = binding
        Path(self.binding["path"]).write_bytes(collector.document(review))
        args = self.calibration_args()
        args.method_review_sha256 = collector.sha256(self.binding["path"])
        with (mock.patch.object(collector, "host_identity", return_value=self.calibration["plan"]["host"]),
              mock.patch.object(collector, "environment", return_value={}),
              mock.patch.object(collector, "binary_identity", return_value=binding),
              mock.patch.object(collector.macos_images, "observe") as observe,
              mock.patch.object(collector, "run_process") as launch,
              self.assertRaisesRegex(ValueError, "timing boundary capability.*zero qualifying launches")):
            collector.collect(args, "calibration")
        observe.assert_not_called()
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_source_host_cost_policy_or_unpinned_review_denies_samples(self):
        original = Path(self.binding["path"]).read_bytes()
        for key in ("host", "baseline", "stage_limits", "policy", "sha256"):
            with self.subTest(key=key):
                review = collector.strict_json(original.decode())
                args = self.calibration_args()
                if key == "sha256":
                    args.method_review_sha256 = "0" * 64
                else:
                    review[key] = {}
                    Path(self.binding["path"]).write_bytes(collector.document(review))
                    args.method_review_sha256 = collector.sha256(self.binding["path"])
                with (mock.patch.object(collector, "host_identity", return_value=self.calibration["plan"]["host"]),
                      mock.patch.object(collector, "environment", return_value={}),
                      mock.patch.object(collector, "binary_identity", return_value=self.identity),
                      mock.patch.object(collector, "run_process") as launch,
                      self.assertRaisesRegex(ValueError, "method review")):
                    collector.collect(args, "calibration")
                launch.assert_not_called()
                Path(self.binding["path"]).write_bytes(original)

    def test_nonempty_eligibility_prose_does_not_replace_the_machine_policy(self):
        review = collector.load(self.binding["path"])
        review.pop("runtime_observer_policy")
        Path(self.binding["path"]).write_bytes(collector.document(review))
        args = self.calibration_args()
        args.method_review_sha256 = collector.sha256(self.binding["path"])
        with (mock.patch.object(collector, "host_identity", return_value=self.calibration["plan"]["host"]),
              mock.patch.object(collector, "environment", return_value={}),
              mock.patch.object(collector, "binary_identity", return_value=self.identity),
              mock.patch.object(collector.macos_images, "observe") as observe,
              mock.patch.object(collector, "run_process") as launch,
              self.assertRaisesRegex(ValueError, "runtime_observer_policy mismatch")):
            collector.collect(args, "calibration")
        observe.assert_not_called()
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_rejected_or_unresolved_method_review_cannot_authorize_collection(self):
        original = collector.load(self.binding["path"])
        for decision in (None, "rejected", "deferred"):
            with self.subTest(decision=decision):
                review = {**original, "decision": decision}
                Path(self.binding["path"]).write_bytes(collector.document(review))
                with self.assertRaises(ValueError):
                    collector.reviewed_method(self.binding["path"], collector.sha256(self.binding["path"]),
                                              self.calibration["plan"])

    def test_control_denies_missing_calibration_runtime_before_creating_campaign(self):
        calibration = copy.deepcopy(self.calibration)
        calibration["prerequisite_issues"] = [
            {"attempt": "runtime:baseline", "status": "incomplete", "reason": "retained runtime bundle absent"}]
        args = collector.parser().parse_args([
            "control", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--baseline", "baseline", "--baseline-metadata", "build.json", "--exclusive-host",
            "--calibration", str(self.root / "calibration"),
            "--calibration-plan-sha256", calibration["plan_sha256"],
            "--method-review", self.binding["path"], "--method-review-sha256", self.binding["sha256"],
            "--output", str(self.root / "control")])
        current_runtime = {"plan": {"host": calibration["plan"]["host"], "environment": {}},
                           "raw": b"{}", "closure": {}}
        with (mock.patch.object(collector, "host_identity", return_value=calibration["plan"]["host"]),
              mock.patch.object(collector, "environment", return_value={}),
              mock.patch.object(collector, "binary_identity", return_value=self.identity),
              mock.patch.object(collector, "evaluate", return_value=calibration),
              mock.patch.object(collector, "runtime_closure_admission", return_value=current_runtime),
              mock.patch.object(collector, "check_files"),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector.macos_images, "observe", side_effect=AssertionError("native launch forbidden")),
              self.assertRaises(ValueError)):
            collector.collect(args, "control")
        self.assertFalse(Path(args.output).exists())

    def test_control_report_retains_calibration_prerequisite_and_known_failures(self):
        calibration = copy.deepcopy(self.calibration)
        calibration["prerequisite_issues"] = [
            {"attempt": "runtime:baseline", "status": "incomplete", "reason": "retained runtime bundle absent"}]
        for numeric_failure, expected in ((False, "incomplete"), (True, "failed")):
            with self.subTest(numeric_failure=numeric_failure):
                control = v2_parsed_evidence(self.root, "control")
                control["plan"]["runtime_observer_policy"] = self.calibration["plan"]["runtime_observer_policy"]
                control["plan"]["binaries"] = {"baseline": self.identity, "control": self.identity}
                control["plan"]["method_review"] = self.binding
                control["plan"]["calibration"] = {"path": str(self.root / "calibration"),
                                                   **collector.cohort_binding(calibration)}
                if numeric_failure:
                    control["observations"]["control"]["single-small96x64"][-1]["peak_rss_bytes"] *= 100
                with mock.patch.object(collector, "evaluate", side_effect=[control, calibration]):
                    result = collector.report(self.root / "control")
                self.assertEqual(result["status"], expected)
                self.assertFalse(result["performance_qualified"])

    def test_unchanged_control_cannot_launch_from_endpoint_only_calibration(self):
        args = collector.parser().parse_args([
            "control", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--baseline", "baseline", "--baseline-metadata", "build.json", "--exclusive-host",
            "--calibration", str(self.root / "calibration"), "--calibration-plan-sha256", self.calibration["plan_sha256"],
            "--method-review", self.binding["path"], "--method-review-sha256", self.binding["sha256"],
            "--output", str(self.root / "control")])
        with (mock.patch.object(collector, "host_identity", return_value=self.calibration["plan"]["host"]),
              mock.patch.object(collector, "environment", return_value={}),
              mock.patch.object(collector, "binary_identity", return_value=self.identity),
              mock.patch.object(collector, "evaluate", return_value=self.calibration),
              mock.patch.object(collector, "run_process") as launch,
              self.assertRaisesRegex(ValueError, "missing runtime closure")):
            collector.collect(args, "control")
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_even_reviewed_numerical_profile_needs_actual_runtime_coverage(self):
        control = v2_parsed_evidence(self.root, "control")
        control["plan"]["runtime_observer_policy"] = self.calibration["plan"]["runtime_observer_policy"]
        control["plan"]["binaries"] = {"baseline": self.identity, "control": self.identity}
        control["plan"]["method_review"] = self.binding
        review = {"format_version": 2, "kind": "component-profile-review", "decision": "accepted",
                  "target": TARGET, "build_mode": "normal",
                  "method_review": self.binding, "calibration": collector.cohort_binding(self.calibration),
                  "control": collector.cohort_binding(control), "envelopes": collector.derive_envelopes(self.calibration),
                  "reviewer": "fixture reviewer", "rationale": "synthetic arithmetic only"}
        review["runtime_observer_policy"] = self.calibration["plan"]["runtime_observer_policy"]
        path = self.root / "profile-review.json"
        collector.save(path, review)
        with self.assertRaisesRegex(ValueError, "missing runtime closure"):
            collector.profile_from_v2(self.calibration, self.root / "calibration", control, self.root / "control",
                                      "reviewer", "finite method", path, collector.sha256(path))
        review["envelopes"]["single-small96x64"]["limits"]["rss_growth_bytes"] += 1
        path.write_bytes(collector.document(review))
        with self.assertRaisesRegex(ValueError, "exact method/cohorts/envelopes"):
            collector.profile_from_v2(self.calibration, self.root / "calibration", control, self.root / "control",
                                      "reviewer", "finite method", path, collector.sha256(path))


class ProtocolTwoControlTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.calibration = v2_parsed_evidence(self.root, "calibration")
        self.control = v2_parsed_evidence(self.root, "control")
        self.profile = v2_numeric_profile(self.calibration)
        self.comparison = v2_parsed_evidence(self.root, "comparison")

    def test_six_baseline_calibrations_and_six_balanced_independent_pairs(self):
        rows = ["single-small96x64", "batch8-cold-full1080p"]
        calibration = collector.schedule("calibration", rows, 2)
        self.assertEqual(len(calibration), 1 + 6 * len(rows))
        self.assertEqual({task["role"] for task in calibration}, {"baseline"})
        for mode, other in (("control", "control"), ("comparison", "candidate")):
            tasks = collector.schedule(mode, rows, 2)
            self.assertEqual(len(tasks), 2 + 12 * len(rows))
            for workload in rows:
                measured = [task for task in tasks if task["workload"] == workload]
                self.assertEqual([task["role"] for task in measured],
                                 [role for index in range(6) for role in
                                  (("baseline", other) if index % 2 == 0 else (other, "baseline"))])
                self.assertEqual([task["position"] for task in measured], [0, 1] * 6)
                self.assertEqual(len({task["id"] for task in measured}), 12)
        self.assertEqual(collector.control_evidence(self.calibration, self.control), [])

    def test_independent_total_campaign_output_and_attempt_bounds_are_fixed(self):
        for build_mode, count in (("normal", 18), ("instrumented", 21)):
            for mode, manifests, per_row, seconds in (
                    ("calibration", 1, 6, 21600), ("control", 2, 12, 43200),
                    ("comparison", 2, 12, 43200), ("smoke", 1, 1, 900)):
                with self.subTest(build_mode=build_mode, mode=mode):
                    specification = v2_plan(self.root, mode, build_mode)
                    limits = specification["execution_limits"]
                    self.assertEqual(limits["max_attempts"], manifests + per_row * count)
                    self.assertEqual(limits["campaign_output_limit_bytes"],
                                     limits["max_attempts"] * 8 * collector.MIB)
                    self.assertEqual(limits["campaign_timeout_seconds"], seconds)
                    self.assertEqual(limits["child_timeout_seconds"], 90 if mode == "smoke" else 300)
                    self.assertEqual(limits["cleanup_seconds"], 5)

    def test_normal_profile_never_fits_heap_or_work_and_keeps_native_growth_slack(self):
        for workload, envelope in self.profile["envelopes"].items():
            self.assertFalse(any("heap" in key for key in envelope["limits"]))
            self.assertEqual(envelope["limits"]["rss_growth_bytes"], collector.MIB)
            self.assertEqual(envelope["limits"]["rss_residual_bytes"], collector.MIB)
        instrumented = v2_numeric_profile(v2_parsed_evidence(self.root, "calibration", "instrumented"))
        limits = instrumented["envelopes"]["batch8-cold-full1080p"]["limits"]
        self.assertEqual(limits["heap_growth_bytes"], 256 * 1024)
        self.assertEqual(limits["rss_growth_bytes"], collector.MIB)
        self.assertIn("peak_heap_bytes", limits)

    def test_sixth_process_and_missing_values_cannot_be_omitted_or_fitted_as_zero(self):
        for missing in ("process", "peak_rss_bytes", "cpu_ns"):
            with self.subTest(missing=missing):
                evidence = copy.deepcopy(self.calibration)
                rows = evidence["observations"]["baseline"]["single-small96x64"]
                if missing == "process":
                    rows.pop()
                else:
                    rows[-1][missing] = None
                with self.assertRaisesRegex(ValueError, "requires 6|unavailable calibration"):
                    collector.derive_envelopes(evidence)

    def test_control_noise_order_drift_and_each_process_ceiling_are_independent(self):
        workload = "selection-k16-dense1080p-keepall"
        for kind in ("order", "drift", "ceiling", "noise"):
            with self.subTest(kind=kind):
                changed = copy.deepcopy(self.control)
                rows = changed["observations"]["control"][workload]
                if kind == "ceiling":
                    rows[-1]["p95_ns"] = self.profile["envelopes"][workload]["limits"]["p95_ns"] + 1
                elif kind == "noise":
                    rows[-1]["p95_ns"] *= 2
                else:
                    for row in rows:
                        if (row["position"] == 0 if kind == "order" else row["process"] >= 3):
                            row["p95_ns"] -= 100
                issues = collector.control_evidence(self.calibration, changed)
                self.assertTrue(any(issue["status"] == "failed" for issue in issues))
                self.assertTrue(any(kind in issue["reason"] or (kind == "ceiling" and "exceeds" in issue["reason"])
                                    for issue in issues))

    def test_control_between_role_bias_uses_frozen_allowance_in_both_directions(self):
        workload = "single-small96x64"
        for metric in ("p95_ns", "cpu_ns"):
            calibration = copy.deepcopy(self.calibration)
            for row in calibration["observations"]["baseline"][workload]:
                row[metric] = 100_000
            allowance = collector.derive_envelopes(calibration)[workload]["benefit_allowances"][metric]
            for role in ("baseline", "control"):
                for excess in (0, 1):
                    with self.subTest(metric=metric, role=role, excess=excess):
                        control = copy.deepcopy(self.control)
                        for label in ("baseline", "control"):
                            for row in control["observations"][label][workload]:
                                row[metric] = 100_000 - (allowance + excess if label == role else 0)
                        issues = collector.control_evidence(calibration, control)
                        self.assertEqual(bool(issues), bool(excess))
                        self.assertTrue(all(issue["status"] == "failed" for issue in issues))

    def test_comparison_noise_is_role_local_and_preserves_missing_data(self):
        workload = "single-small96x64"
        for metric in ("p95_ns", "cpu_ns"):
            calibration = copy.deepcopy(self.calibration)
            for row in calibration["observations"]["baseline"][workload]:
                row[metric] = 100_000
            profile = v2_numeric_profile(calibration)
            for role in ("baseline", "candidate"):
                for excess, missing in ((0, False), (1, False), (1, True)):
                    with self.subTest(metric=metric, role=role, excess=excess, missing=missing):
                        comparison = copy.deepcopy(self.comparison)
                        other = "candidate" if role == "baseline" else "baseline"
                        for row in comparison["observations"][other][workload]:
                            row[metric] = 100_000
                        for index, row in enumerate(comparison["observations"][role][workload]):
                            row[metric] = 80_000 if index < 4 else 92_000 + excess
                        if missing:
                            comparison["observations"][other][workload].pop()
                            comparison["observations"][role][workload][0]["peak_rss_bytes"] = None
                        issues, _ = collector.compare_evidence(comparison, profile)
                        self.assertEqual(any(issue["status"] == "failed" for issue in issues), bool(excess))
                        self.assertEqual(any(issue["status"] == "incomplete" for issue in issues), missing)

    def test_control_rejects_rebinding_to_another_calibration_receipt(self):
        for field in ("plan_sha256", "attempts"):
            with self.subTest(field=field):
                calibration = copy.deepcopy(self.calibration)
                if field == "plan_sha256":
                    calibration[field] = "7" * 64
                else:
                    calibration[field][0]["sha256"] = "7" * 64
                with self.assertRaisesRegex(ValueError, "calibration receipts"):
                    collector.control_evidence(calibration, self.control)

    def test_unrelated_missing_metric_preserves_known_control_order_failure(self):
        workload = "selection-k16-dense1080p-keepall"
        rows = self.control["observations"]["control"][workload]
        for row in rows:
            if row["position"] == 0:
                row["p95_ns"] -= 100
        rows[0]["cpu_ns"] = None
        rows[0]["unavailable"] = ["cpu_ns"]
        issues = collector.control_evidence(self.calibration, self.control)
        self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))
        self.assertTrue(any(issue["status"] == "failed" and "p95_ns order" in issue["reason"] for issue in issues))

    def test_control_missing_row_cannot_hide_known_resource_failure(self):
        workload = "batch8-cold-full1080p"
        self.control["observations"]["baseline"][workload].pop()
        self.control["observations"]["control"][workload][-1]["rss_growth_bytes"] = 2 * collector.MIB
        issues = collector.control_evidence(self.calibration, self.control)
        self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))
        self.assertTrue(any(issue["status"] == "failed" and "rss_growth_bytes" in issue["reason"] for issue in issues))

    def test_mixed_mode_source_host_method_or_shared_cohort_is_rejected(self):
        mutations = (
            lambda value: value["plan"].update(build_mode="instrumented"),
            lambda value: value["plan"]["host"].update(node="another-host"),
            lambda value: value["plan"]["method_review"].update(sha256="0" * 64),
            lambda value: value["plan"]["binaries"]["baseline"]["build"].update(source_tree="0" * 40),
            lambda value: value.update(plan_sha256=self.calibration["plan_sha256"]),
            lambda value: value.update(attempts=copy.deepcopy(self.calibration["attempts"])),
        )
        for mutate in mutations:
            changed = copy.deepcopy(self.control)
            mutate(changed)
            with self.assertRaises(ValueError):
                collector.control_evidence(self.calibration, changed)

    def test_control_executable_substitution_is_not_an_unchanged_control(self):
        self.control["plan"]["binaries"]["control"] = v2_identity(self.root, "normal", "candidate")
        with self.assertRaisesRegex(ValueError, "control executable/source"):
            collector.control_evidence(self.calibration, self.control)

    def test_control_failure_denies_profile_and_candidate_launch(self):
        self.control["observations"]["control"]["single-small96x64"][0]["rss_residual_bytes"] = 2 * collector.MIB
        with self.assertRaisesRegex(ValueError, "A/A control failed"):
            collector.profile_from_v2(self.calibration, self.root / "calibration", self.control,
                                      self.root / "control", "reviewer", "finite method", None, None)
        # A retained profile must be reconstructed from its pinned control cohort,
        # even when the supplied JSON calls itself frozen and accepted.
        profile = {**self.profile, "kind": "frozen-profile",
                   "calibration": {"path": str(self.root / "calibration")},
                   "control": {"path": str(self.root / "control")}, "profile_review": {},
                   "reviewer": "reviewer", "rationale": "fixed method"}
        path = self.root / "profile.json"
        collector.save(path, profile)
        args = collector.parser().parse_args([
            "compare", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--baseline", "baseline", "--baseline-metadata", "build.json",
            "--candidate", "candidate", "--candidate-metadata", "candidate.json", "--exclusive-host",
            "--profile", str(path), "--profile-sha256", collector.sha256(path),
            "--output", str(self.root / "comparison")])
        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "binary_identity", return_value=v2_identity(self.root)),
              mock.patch.object(collector, "evaluate", side_effect=[self.calibration, self.control]),
              mock.patch.object(collector, "run_process") as launch,
              self.assertRaisesRegex(ValueError, "A/A control failed")):
            collector.collect(args, "comparison")
        launch.assert_not_called()
        self.assertFalse((self.root / "comparison").exists())

    def test_normal_benefit_requires_joint_cpu_and_p95_on_each_primary_row(self):
        self.assertEqual(collector.compare_evidence(self.comparison, self.profile)[0], [])
        for workload in ("selection-k16-dense1080p-keepall", "batch8-cold-full1080p"):
            for metric in ("p95_ns", "cpu_ns"):
                changed = copy.deepcopy(self.comparison)
                baseline = changed["observations"]["baseline"][workload][0][metric]
                allowance = self.profile["envelopes"][workload]["benefit_allowances"][metric]
                for row in changed["observations"]["candidate"][workload]:
                    row[metric] = baseline - allowance
                issues, benefits = collector.compare_evidence(changed, self.profile)
                self.assertFalse(benefits[workload][metric]["passed"])
                self.assertTrue(any(f"{metric} benefit" in issue["reason"] for issue in issues))

    def test_each_sixth_process_guard_survives_an_improved_median(self):
        workload = "selection-k16-dense1080p-keepall"
        for role in ("baseline", "candidate"):
            changed = copy.deepcopy(self.comparison)
            changed["observations"][role][workload][-1]["rss_residual_bytes"] = 2 * collector.MIB
            issues, benefits = collector.compare_evidence(changed, self.profile)
            self.assertTrue(benefits[workload]["p95_ns"]["passed"])
            self.assertTrue(any(issue["attempt"] == f"{role}:{workload}:5" and "rss_residual_bytes" in issue["reason"]
                                for issue in issues))

    def test_instrumented_counters_cannot_substitute_for_normal_benefit(self):
        calibration = v2_parsed_evidence(self.root, "calibration", "instrumented")
        comparison = v2_parsed_evidence(self.root, "comparison", "instrumented")
        profile = v2_numeric_profile(calibration)
        issues, benefits = collector.compare_evidence(comparison, profile)
        self.assertEqual(issues, [])
        self.assertEqual(benefits, {})
        with self.assertRaisesRegex(ValueError, "build_mode"):
            collector.compare_evidence(comparison, self.profile)

    def test_profile_cannot_change_mode_or_envelopes_without_new_review_bytes(self):
        path = self.root / "profile.json"
        collector.save(path, self.profile)
        original = path.read_bytes()
        changed = copy.deepcopy(self.profile)
        changed["envelopes"]["single-small96x64"]["limits"]["rss_growth_bytes"] += 1
        with self.assertRaises(FileExistsError):
            collector.save(path, changed)
        self.assertEqual(path.read_bytes(), original)
        # The actual frozen-profile constructor needs independent control plus a
        # byte-pinned numerical review; reviewer prose alone grants no authority.
        with self.assertRaisesRegex(ValueError, "profile_review_sha256"):
            collector.profile_from_v2(self.calibration, self.root / "calibration", self.control,
                                      self.root / "control", "reviewer", "finite method", None, None)


class NativeThreadObservationTests(unittest.TestCase):
    def test_smoke_cannot_accept_unavailable_or_changed_thread_observation(self):
        with tempfile.TemporaryDirectory() as temporary:
            for phase, change in (("manifest", "unavailable"), ("workload", "drift")):
                with self.subTest(phase=phase):
                    directory = Path(temporary) / phase
                    specification = persist_v2_smoke(directory)
                    task = specification["schedule"][0 if phase == "manifest" else 1]
                    path = directory / "attempts" / f"{task['id']}.json"
                    attempt = collector.load(path)
                    value = collector.strict_json(attempt["record"]["stdout"])
                    observed = value["opencv_parallel_threads"]
                    if change == "unavailable":
                        observed["before"] = {"framework": None, "threads": None, "reason": "getter unavailable"}
                        observed["after"] = copy.deepcopy(observed["before"])
                    else:
                        observed["after"]["threads"] += 1
                        observed["changed"] = True
                    attempt["record"]["stdout"] = json.dumps(value)
                    path.write_bytes(collector.document(attempt))
                    result = collector.report(directory)
                    self.assertNotEqual(result["status"], "passed")
                    self.assertFalse(result["performance_qualified"])

    def test_comparison_rejects_different_runtime_settings_between_roles(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            calibration = v2_parsed_evidence(root, "calibration")
            profile = v2_numeric_profile(calibration)
            for field, value in (("threads", 8), ("framework", "another-runtime")):
                with self.subTest(field=field):
                    comparison = v2_parsed_evidence(root, "comparison")
                    comparison["native_threads"]["candidate"][field] = value
                    issues, _ = collector.compare_evidence(comparison, profile)
                    self.assertTrue(any(issue.get("category") == "native-threads"
                                        and issue["status"] == "failed" for issue in issues))

    def test_role_setting_mismatch_stops_before_any_process_or_output(self):
        class ReachedLaunch(Exception):
            pass

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for name, change in (("equal", {}), ("threads", {"threads": 8}),
                                 ("framework", {"framework": "another-runtime"})):
                with self.subTest(name=name):
                    specification = v2_plan(root, "comparison")
                    specification["runtime_observer_policy"] = "macos-dyld-observer-on-warm-v1"
                    specification["binaries"] = {
                        role: timing_identity(binary) for role, binary in specification["binaries"].items()}
                    probe = specification["binaries"]["candidate"]["build"]["provenance_commands"][0]["record"]
                    value = collector.strict_json(probe["stdout"])
                    for point in ("before", "after"):
                        value["opencv_parallel_threads"][point].update(change)
                    probe["stdout"] = json.dumps(value)
                    inputs = root / f"inputs-{name}"
                    inputs.mkdir()
                    method = save_method_review(inputs, specification)
                    profile = v2_numeric_profile(v2_parsed_evidence(root, "calibration"))
                    profile.update(method_review=method, calibration={"path": str(root / "calibration")},
                                   control={"path": str(root / "control")})
                    profile.update(baseline=specification["binaries"]["baseline"],
                                   runtime_observer_policy=specification["runtime_observer_policy"])
                    profile_path = inputs / "profile.json"
                    collector.save(profile_path, profile)
                    args = collector.parser().parse_args([
                        "compare", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
                        "--baseline", "baseline", "--baseline-metadata", "base.json",
                        "--candidate", "candidate", "--candidate-metadata", "candidate.json",
                        "--profile", str(profile_path), "--profile-sha256", collector.sha256(profile_path),
                        "--method-review", method["path"], "--method-review-sha256", method["sha256"],
                        "--exclusive-host", "--output", str(root / name)])
                    base = {"plan": {key: specification[key] for key in ("host", "environment")},
                            "threads": collector.effective_threads(v2_manifest()), "closure": {}, "raw": b"{}"}
                    candidate = copy.deepcopy(base)
                    candidate["threads"] = collector.effective_threads(value)
                    with (mock.patch.object(collector, "host_identity", return_value=specification["host"]),
                          mock.patch.object(collector, "environment", return_value=specification["environment"]),
                          mock.patch.object(collector, "binary_identity",
                                            side_effect=list(specification["binaries"].values())),
                          mock.patch.object(collector, "validate_profile", return_value=profile),
                          mock.patch.object(collector, "runtime_closure_admission", side_effect=[base, candidate]),
                          mock.patch.object(collector, "check_files"),
                          mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
                          mock.patch.object(collector.macos_images, "observe", side_effect=ReachedLaunch) as launch):
                        with self.assertRaises(ValueError if change else ReachedLaunch):
                            collector.collect(args, "comparison")
                    self.assertEqual(launch.call_count, 0 if change else 1)
                    self.assertEqual(Path(args.output).exists(), not bool(change))

    def test_missing_unsupported_or_changed_runtime_setting_is_unavailable(self):
        for change in ("method", "zero", "boolean", "reason", "missing_reason", "gcd", "drift", "changed"):
            with self.subTest(change=change):
                value = v2_fields("normal")
                observed = value["opencv_parallel_threads"]
                if change == "method":
                    observed["method"] = "build-configuration-only"
                elif change in ("zero", "boolean"):
                    observed["before"]["threads"] = 0 if change == "zero" else True
                elif change == "reason":
                    observed["before"]["reason"] = "unavailable"
                elif change == "missing_reason":
                    del observed["before"]["reason"]
                elif change == "gcd":
                    observed["before"]["framework"] = "GcD"
                elif change == "drift":
                    observed["after"]["threads"] += 1
                else:
                    observed["changed"] = True
                if change not in ("drift", "changed"):
                    observed["after"] = copy.deepcopy(observed["before"])
                with self.assertRaises(ValueError):
                    collector.effective_threads(value)


class WindowsRuntimeAttemptTests(unittest.TestCase):
    def setUp(self):
        from tools.tests import test_windows_loader_observer as native
        threads = {"framework": "ms-concurrency", "threads": 20, "reason": None}
        record = native.raw_record()
        value = json.loads(record["stdout"])
        value["opencv_parallel_threads"] = {
            "method": collector.OPENCV_THREAD_METHOD, "before": threads, "after": threads, "changed": False}
        record["stdout"] = json.dumps(value)
        discovery, _, _ = native.run_observer(record=record)
        closure = native.observer.validate_closure([discovery])["closure"]
        receipt, _, _ = native.run_observer(closure=closure, record=record)
        self.attempt = {"task": {"workload": "hit-full"}, "record": receipt["record"],
                        "runtime_observation": {key: item for key, item in receipt.items() if key != "record"},
                        "timeout_seconds": 300, "cleanup_seconds": 5, "runtime_binding_seconds": 120}
        self.runtime = {"plan": {"target": collector.TARGETS[1], "observer": receipt["method_identity"],
                                "binary": {"path": native.MAIN, "build": {
                                    "dependencies": [{"path": native.NTDLL}]}}},
                        "closure": closure, "threads": threads}

    def test_equivalent_windows_paths_pass_but_other_images_refuse(self):
        binary = self.runtime["plan"]["binary"]
        binary["path"] = r"\\?\c:\APP\BENCH.EXE"
        binary["build"]["dependencies"][0]["path"] = r"\\?\c:\WINDOWS\SYSTEM32\NTDLL.DLL"
        original = copy.deepcopy(self.attempt)
        collector.validate_runtime_attempt(self.attempt, self.runtime)
        self.assertEqual(self.attempt, original)
        for field in ("executable", "dependency"):
            with self.subTest(field=field):
                other = copy.deepcopy(self.runtime)
                selected = other["plan"]["binary"]
                if field == "dependency":
                    selected = selected["build"]["dependencies"][0]
                selected["path"] = r"\\?\C:\unobserved\other.dll"
                with self.assertRaisesRegex(ValueError, "omits an attested image"):
                    collector.validate_runtime_attempt(self.attempt, other)
        tampered = copy.deepcopy(self.attempt)
        tampered["runtime_observation"]["images"][0]["sha256"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "runtime observation incomplete"):
            collector.validate_runtime_attempt(tampered, self.runtime)


@unittest.skipIf(sys.platform == "win32", "retained Mach-O fixture uses POSIX image paths")
class RuntimePreflightTests(unittest.TestCase):
    def setUp(self):
        from tools.tests import test_macos_image_observer as native
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.native = native
        self.run = native.SyntheticRun(self.root)
        self.run.lib = self.run.lib.rename(self.root / "libopencv_core.fixture.dylib")
        self.identity = v2_identity(self.root)
        self.identity.update(path=str(self.run.main), sha256=collector.sha256(self.run.main))
        build = self.identity["build"]
        build["binary_sha256"] = self.identity["sha256"]
        build["dependencies"] = [{"path": str(self.run.lib), "sha256": collector.sha256(self.run.lib)}]
        self.runtime = {"method": RUNTIME["method"], "images": copy.deepcopy(build["dependencies"])}
        declaration = v2_manifest()
        declaration["loaded_native_runtime"] = self.runtime
        build["benchmark_manifest"] = collector.manifest_identity(declaration, TARGET, "4.12.0", 2, "normal")
        build["provenance_commands"][0] = {
            "argv": [self.identity["path"], "--protocol-version", "2", "--list"],
            "record": process_record(declaration)}
        tasks = collector.schedule("smoke", list(collector.definitions_for("normal")), 2)
        specification = {"target": TARGET, "build_mode": "normal", "binary": self.identity,
                         "host": {"target": TARGET}, "environment": {"DYLD_LIBRARY_PATH": "/opt/lib"},
                         "collector": collector.collector_identity(), "observer": collector.macos_images.method_identity(),
                         "schedule": tasks, "execution_limits": {
                             **collector.execution_limits("smoke", len(tasks)), "runtime_binding_seconds": 120}}
        attempts = []
        for index, task in enumerate(tasks):
            value = copy.deepcopy(declaration) if task["workload"] is None else v2_payload(task["workload"], smoke=True)
            value["loaded_native_runtime"] = self.runtime
            receipt = self.run.discovery(stdout=json.dumps(value))
            receipt["duration_seconds"] = 0.2
            attempts.append({"task": task, "argv": collector.command(self.identity, task, "smoke", 2),
                             "identity_before": self.identity, "identity_after": self.identity,
                             "identity_error": None, "interruption": None, "receipt": receipt,
                             "campaign_elapsed_before": index, "campaign_elapsed_after": index + 0.3,
                             "timeout_seconds": 90})
        self.bundle = {"format_version": 2, "kind": "component-runtime-preflight", "plan": specification,
                       "attempts": attempts, "threads": collector.effective_threads(declaration), "closure": None}
        self.close(self.bundle)

    def close(self, bundle):
        derived = collector.macos_images.validate_closure([item["receipt"] for item in bundle["attempts"]])
        self.assertTrue(derived["ok"], derived["reason"])
        bundle["closure"] = derived["closure"]

    def validate(self, bundle=None):
        return collector.validate_runtime_bundle(bundle or self.bundle, self.identity, TARGET, "normal")

    def enable_timing(self):
        capable = timing_identity(self.identity)
        self.identity.clear()
        self.identity.update(capable)
        self.bundle["plan"]["runtime_observer_policy"] = "macos-dyld-observer-on-warm-v1"
        for attempt in self.bundle["attempts"]:
            task = attempt["task"]
            value = collector.strict_json(attempt["receipt"]["record"]["stdout"])
            value.update(timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=True)
            log = self.run.log()
            if task["workload"] is not None:
                log += self.native.timing_row(task["workload"])
            attempt["receipt"] = self.run.discovery(log, stdout=json.dumps(value))
            attempt["receipt"]["duration_seconds"] = 0.2
            attempt["argv"] = collector.command(self.identity, task, "smoke", 2, timing_boundaries=True)
            collector.validate_timing_attempt(
                attempt, self.identity["build"]["benchmark_manifest"], attempt["receipt"]["record"], retain=True)
        self.close(self.bundle)

    def test_incapable_preflight_refuses_before_creating_output_or_launching(self):
        args = collector.parser().parse_args([
            "runtime-preflight", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--binary", "fixture", "--build-metadata", "fixture.json", "--output", str(self.root / "incapable")])
        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "binary_identity", return_value=self.identity),
              mock.patch.object(collector.macos_images, "discover") as discover,
              self.assertRaisesRegex(ValueError, "timing boundary capability.*zero qualifying launches")):
            collector.runtime_preflight(args)
        discover.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_historical_image_only_bundle_cannot_authorize_the_warm_policy(self):
        path = self.root / "image-only.json"
        collector.save(path, self.bundle)
        binding = copy.deepcopy(self.identity)
        binding["build"]["runtime_image_preflight"] = {"path": str(path), "sha256": collector.sha256(path)}
        collector.runtime_closure_admission(binding, TARGET, "normal")
        with self.assertRaisesRegex(ValueError, "timing boundary capability"):
            collector.runtime_closure_admission(
                binding, TARGET, "normal", timing_policy="macos-dyld-observer-on-warm-v1")

    def test_capable_image_only_preflight_does_not_imply_the_warm_policy(self):
        capable = timing_identity(self.identity)
        self.identity.clear()
        self.identity.update(capable)
        for attempt in self.bundle["attempts"]:
            value = collector.strict_json(attempt["receipt"]["record"]["stdout"])
            value.update(timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=False)
            attempt["receipt"]["record"]["stdout"] = json.dumps(value)
        self.close(self.bundle)
        path = self.root / "capable-image-only.json"
        collector.save(path, self.bundle)
        binding = copy.deepcopy(self.identity)
        binding["build"]["runtime_image_preflight"] = {"path": str(path), "sha256": collector.sha256(path)}
        collector.runtime_closure_admission(binding, TARGET, "normal")
        with self.assertRaisesRegex(ValueError, "timing boundary preflight policy"):
            collector.runtime_closure_admission(
                binding, TARGET, "normal", timing_policy="macos-dyld-observer-on-warm-v1")

    def test_preflight_summary_cannot_hide_missing_or_late_raw_markers(self):
        self.enable_timing()
        self.validate()
        original = self.bundle["attempts"][1]
        task = original["task"]
        stdout = original["receipt"]["record"]["stdout"]
        duplicate = self.native.load_line(7, self.native.UUID_B, str(self.run.lib))
        for name, log in (
                ("missing", self.run.log()),
                ("duplicate", self.run.log() + self.native.timing_row(task["workload"], inside=duplicate * 2)),
                ("wrong-row", self.run.log() + self.native.timing_row("other-row")),
                ("foreign-pid", self.run.log() + self.native.timing_row(task["workload"]).replace("[7]", "[8]"))):
            with self.subTest(name=name):
                broken = copy.deepcopy(self.bundle)
                broken["attempts"][1]["receipt"] = self.run.discovery(log, stdout=stdout)
                broken["attempts"][1]["receipt"]["duration_seconds"] = 0.2
                self.close(broken)
                with self.assertRaisesRegex(ValueError, "timing observation summary mismatch"):
                    self.validate(broken)

    def test_capability_is_not_invocation_enablement_or_a_blank_window(self):
        self.enable_timing()
        original = self.bundle["attempts"][1]
        manifest = self.identity["build"]["benchmark_manifest"]
        for change in ("disabled", "missing-enabled", "unsupported", "missing-flag", "duplicate-flag",
                       "wrong-output-row", "missing-pid", "forged-count", "missing-summary"):
            with self.subTest(change=change):
                attempt = copy.deepcopy(original)
                record = attempt["receipt"]["record"]
                value = collector.strict_json(record["stdout"])
                if change == "disabled":
                    value["timing_boundaries_enabled"] = False
                elif change == "missing-enabled":
                    del value["timing_boundaries_enabled"]
                elif change == "unsupported":
                    value["timing_boundary_protocol"] = "mado-component-timing-v2"
                elif change == "missing-flag":
                    attempt["argv"].remove("--timing-boundaries")
                elif change == "duplicate-flag":
                    attempt["argv"].append("--timing-boundaries")
                elif change == "wrong-output-row":
                    value["workloads"][0]["id"] = "other-row"
                elif change == "missing-pid":
                    record.pop("process_id")
                elif change == "forged-count":
                    attempt["timing_observation"]["rows"][0]["in_window_dyld_records"] = False
                else:
                    del attempt["timing_observation"]
                record["stdout"] = json.dumps(value)
                with self.assertRaises(ValueError):
                    collector.validate_timing_attempt(attempt, manifest, record)

    def test_nonstatistical_rows_keep_counts_but_still_require_complete_markers(self):
        self.enable_timing()
        manifest = self.identity["build"]["benchmark_manifest"]
        workload = next(row["id"] for row in manifest["workloads"] if not row["statistical"])
        attempt = copy.deepcopy(next(item for item in self.bundle["attempts"] if item["task"]["workload"] == workload))
        duplicate = self.native.load_line(7, self.native.UUID_B, str(self.run.lib))
        record = attempt["receipt"]["record"]
        record["stderr"] = self.run.log() + self.native.timing_row(workload, inside=duplicate * 2)
        collector.validate_timing_attempt(attempt, manifest, record, retain=True)
        self.assertFalse(attempt["timing_observation"]["ok"])
        self.assertEqual(attempt["timing_observation"]["rows"][0]["in_window_dyld_records"], 2)
        record["stderr"] = self.run.log()
        with self.assertRaisesRegex(ValueError, "timing_markers_missing"):
            collector.validate_timing_attempt(attempt, manifest, record, retain=True)

    def test_qualifying_failures_keep_post_identity_and_stop_later_rows(self):
        self.enable_timing()
        bundle_path = self.root / "runtime.json"
        collector.save(bundle_path, self.bundle)
        binding = copy.deepcopy(self.identity)
        binding["build"]["runtime_image_preflight"] = {
            "path": str(bundle_path), "sha256": collector.sha256(bundle_path)}
        metadata = self.root / "build.json"
        collector.save(metadata, binding["build"])
        binding.update(metadata_path=str(metadata), metadata_sha256=collector.sha256(metadata))
        specification = v2_plan(self.root, "calibration")
        specification.update(binaries={"baseline": binding}, host=self.bundle["plan"]["host"],
                             environment=self.bundle["plan"]["environment"])
        specification["runtime_observer_policy"] = "macos-dyld-observer-on-warm-v1"
        method = save_method_review(self.root, specification)
        args = collector.parser().parse_args([
            "calibrate", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--baseline", binding["path"], "--baseline-metadata", str(metadata), "--exclusive-host",
            "--method-review", method["path"], "--method-review-sha256", method["sha256"],
            "--output", str(self.root / "campaign")])
        launched = []

        def observe(argv, **options):
            task = specification["schedule"][len(launched)]
            launched.append(task)
            self.assertEqual(argv.count("--timing-boundaries"), 1)
            if task["workload"] is None:
                value = collector.strict_json(binding["build"]["provenance_commands"][0]["record"]["stdout"])
            else:
                value = v2_payload(task["workload"])
                value["loaded_native_runtime"] = self.runtime
            value.update(timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=True)
            log = self.run.log()
            if task["workload"] is not None:
                inside = (self.native.load_line(7, self.native.UUID_B, str(self.run.lib)) * 2
                          if failure == "in-window" else "")
                if failure != "missing":
                    log += self.native.timing_row(task["workload"], inside=inside)
                if failure == "oracle":
                    value["workloads"][0]["oracle_failures"] = 1
                    value["workloads"][0]["samples"][0]["correct"] = False
            receipt = self.run.observation(options["closure"], log, stdout=json.dumps(value),
                                           exit_code=23 if task["id"] == "0001" and failure == "exit" else 0)
            receipt["duration_seconds"] = 0.2
            return receipt

        for failure, category in (("exit", "exit"), ("missing", "provenance"),
                                  ("in-window", "provenance"), ("oracle", "semantic")):
            with self.subTest(failure=failure):
                launched.clear()
                args.output = str(self.root / f"campaign-{failure}")
                with (mock.patch.object(collector, "host_identity", return_value=specification["host"]),
                      mock.patch.object(collector, "environment", return_value=specification["environment"]),
                      mock.patch.object(collector, "binary_identity", return_value=binding),
                      mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
                      mock.patch.object(collector.macos_images, "observe", side_effect=observe)):
                    result = collector.collect(args, "calibration")
                failed = collector.load(Path(args.output) / "attempts/0001.json")
                self.assertEqual(failed["identity_after"], binding)
                self.assertIsNone(failed["identity_error"])
                self.assertEqual(launched, specification["schedule"][:2])
                self.assertEqual(result["status"], "failed")
                self.assertFalse(result["performance_qualified"])
                self.assertIn(category, {issue.get("category") for issue in result["issues"]})
                self.assertNotIn("identity", {issue.get("category") for issue in result["issues"]})
                self.assertTrue(any(issue["status"] == "not-run" for issue in result["issues"]))

    def test_retained_preflight_admission_needs_no_original_target_files(self):
        path = self.root / "bundle.json"
        collector.save(path, self.bundle)
        augmented = copy.deepcopy(self.identity)
        augmented["build"]["runtime_image_preflight"] = {"path": str(path), "sha256": collector.sha256(path)}
        self.run.main.unlink()
        self.run.lib.unlink()
        result = collector.runtime_closure_admission(augmented, TARGET, "normal")
        self.assertEqual(result["closure"], self.bundle["closure"])
        self.assertEqual(result["threads"], self.bundle["threads"])

    def test_protocol2_producer_and_admission_keep_full_boundary_identities(self):
        self.enable_timing()
        metadata = self.root / "original-build.json"
        collector.save(metadata, self.identity["build"])
        destination = self.root / "protocol2"
        args = collector.parser().parse_args([
            "runtime-preflight", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--binary", str(self.run.main), "--build-metadata", str(metadata), "--output", str(destination)])
        receipts = [copy.deepcopy(attempt["receipt"]) for attempt in self.bundle["attempts"]]
        with (mock.patch.object(collector, "host_identity", return_value=self.bundle["plan"]["host"]),
              mock.patch.object(collector, "environment", return_value=self.bundle["plan"]["environment"]),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector.macos_images, "discover", side_effect=receipts)):
            result = collector.runtime_preflight(args)
        self.assertEqual(result["status"], "passed", result["issues"])
        bundle = collector.load(destination / "bundle.json")
        for attempt in bundle["attempts"]:
            self.assertEqual(attempt["identity_before"], bundle["plan"]["binary"])
            self.assertEqual(attempt["identity_after"], bundle["plan"]["binary"])
        identity = collector.binary_identity(self.run.main, result["metadata"], TARGET)
        admitted = collector.runtime_closure_admission(identity, TARGET, "normal")
        self.assertEqual(admitted["closure"], self.bundle["closure"])
        bundle["attempts"][-1]["identity_after"] = {
            "plan_binary_sha256": hashlib.sha256(collector.document(bundle["plan"]["binary"])).hexdigest()}
        path = self.root / "compact-protocol2.json"
        collector.save(path, bundle)
        identity["build"]["runtime_image_preflight"] = {"path": str(path), "sha256": collector.sha256(path)}
        with self.assertRaisesRegex(ValueError, "runtime preflight file identity"):
            collector.runtime_closure_admission(identity, TARGET, "normal")

    def test_failed_preflight_with_complete_safety_facts_allows_independent_smoke(self):
        self.enable_timing()
        metadata = self.root / "original-build.json"
        collector.save(metadata, self.identity["build"])
        for failure in ("nonzero-exit", "semantic"):
            with self.subTest(failure=failure):
                lock_root = self.root / failure
                lock_root.mkdir()
                destination = lock_root / "preflight"
                args = collector.parser().parse_args([
                    "runtime-preflight", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
                    "--binary", str(self.run.main), "--build-metadata", str(metadata), "--output", str(destination)])
                template = self.bundle["attempts"][1]["receipt"]
                value = collector.strict_json(template["record"]["stdout"])
                if failure == "semantic":
                    value["workloads"][0]["samples"][0]["correct"] = False
                failed = self.run.discovery(
                    template["record"]["stderr"], stdout=json.dumps(value),
                    exit_code=23 if failure == "nonzero-exit" else 0)
                failed["duration_seconds"] = 0.2
                original = copy.deepcopy(failed)
                smoke = collector.parser().parse_args([
                    "smoke", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
                    "--baseline", str(self.run.main), "--baseline-metadata", str(metadata),
                    "--output", str(lock_root / "diagnostic"), "--workload", "single-small96x64"])
                payload = v2_payload("single-small96x64", smoke=True)
                payload.update(loaded_native_runtime=self.runtime,
                               timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=False)
                records = [self.identity["build"]["provenance_commands"][0]["record"], process_record(payload)]
                with (mock.patch.object(collector.tempfile, "gettempdir", return_value=str(lock_root)),
                      mock.patch.object(collector, "host_identity", return_value=self.bundle["plan"]["host"]),
                      mock.patch.object(collector, "environment", return_value=self.bundle["plan"]["environment"]),
                      mock.patch.object(collector.macos_images, "discover", side_effect=[
                          self.bundle["attempts"][0]["receipt"], failed]) as discover):
                    result = collector.runtime_preflight(args)
                    self.assertEqual((result["status"], discover.call_count), ("incomplete", 2))
                    self.assertFalse((destination / "bundle.json").exists())
                    self.assertFalse((lock_root / "mado-template-matching-efficiency.lock").exists())
                    attempt = collector.load(destination / "attempts/0001.json")
                    self.assertTrue(attempt["provenance_complete"])
                    self.assertEqual(attempt["identity_before"], attempt["identity_after"])
                    self.assertEqual(attempt["identity_before"]["sha256"], self.identity["sha256"])
                    self.assertIsNone(attempt["identity_error"])
                    self.assertIsNone(attempt["provenance_error"])
                    self.assertIsNone(attempt["cleanup_error"])
                    field = "execution_error" if failure == "nonzero-exit" else "semantic_error"
                    self.assertIsNotNone(attempt[field])
                    self.assertEqual(attempt["receipt"], original)
                    retained = {path: path.read_bytes() for path in destination.rglob("*.json")}
                    with (mock.patch.object(collector, "run_process", side_effect=records) as launch,
                          mock.patch.object(collector, "reviewed_method", side_effect=AssertionError("no qualification"))):
                        diagnostic = collector.collect(smoke, "smoke")
                    self.assertEqual((diagnostic["status"], launch.call_count), ("passed", 2))
                    self.assertEqual(diagnostic["qualification_status"], "not-requested")
                    self.assertFalse(diagnostic["performance_qualified"])
                    self.assertEqual(retained, {path: path.read_bytes() for path in retained})
                    self.assertFalse((lock_root / "mado-template-matching-efficiency.lock").exists())
                self.assertEqual(failed, original)
                if failure == "nonzero-exit":
                    closure = collector.macos_images.validate_closure(failed)
                    self.assertFalse(closure["ok"])
                    self.assertIsNone(closure["closure"])

    def test_preflight_with_missing_safety_facts_keeps_fence_across_new_smoke_output(self):
        self.enable_timing()
        metadata = self.root / "original-build.json"
        collector.save(metadata, self.identity["build"])
        for failure in ("cleanup", "unknown-cleanup", "source", "foreign-process", "output", "identity", "host-change"):
            with self.subTest(failure=failure):
                lock_root = self.root / failure
                lock_root.mkdir()
                destination = lock_root / "preflight"
                args = collector.parser().parse_args([
                    "runtime-preflight", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
                    "--binary", str(self.run.main), "--build-metadata", str(metadata), "--output", str(destination)])
                index = 1 if failure == "host-change" else 0
                failed = copy.deepcopy(self.bundle["attempts"][index]["receipt"])
                if failure in ("cleanup", "unknown-cleanup"):
                    failed["record"]["cleanup_ok"] = False if failure == "cleanup" else None
                elif failure == "source":
                    failed["images"][0]["sha256"] = "f" * 64
                elif failure == "foreign-process":
                    failed["record"]["process_id"] = 8
                elif failure == "output":
                    failed["record"]["output_limited"] = True
                elif failure == "host-change":
                    failed["record"]["exit_code"] = 23
                    failed["system"]["os_build"] = "26A435"
                else:
                    value = collector.strict_json(failed["record"]["stdout"])
                    value["target"] = "x86_64-pc-windows-msvc"
                    failed["record"]["stdout"] = json.dumps(value)
                receipts = [self.bundle["attempts"][0]["receipt"], failed] if index else [failed]
                smoke = collector.parser().parse_args([
                    "smoke", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
                    "--baseline", str(self.run.main), "--baseline-metadata", str(metadata),
                    "--output", str(lock_root / "diagnostic"), "--workload", "single-small96x64"])
                with (mock.patch.object(collector.tempfile, "gettempdir", return_value=str(lock_root)),
                      mock.patch.object(collector, "host_identity", return_value=self.bundle["plan"]["host"]),
                      mock.patch.object(collector, "environment", return_value=self.bundle["plan"]["environment"]),
                      mock.patch.object(collector.macos_images, "discover", side_effect=receipts) as discover):
                    result = collector.runtime_preflight(args)
                    self.assertEqual((result["status"], discover.call_count), ("incomplete", len(receipts)))
                    self.assertFalse((destination / "bundle.json").exists())
                    attempt = collector.load(destination / f"attempts/{index:04d}.json")
                    self.assertEqual(attempt["receipt"], failed)
                    if failure == "cleanup":
                        self.assertTrue(attempt["provenance_complete"])
                        self.assertIsNotNone(attempt["cleanup_error"])
                    else:
                        self.assertFalse(attempt["provenance_complete"])
                        self.assertIsNotNone(attempt["provenance_error"])
                    if failure == "host-change":
                        self.assertIn("system_differs", attempt["provenance_error"])
                    lock = lock_root / "mado-template-matching-efficiency.lock"
                    retained = lock.read_bytes()
                    with mock.patch.object(collector, "run_process") as launch, self.assertRaises(ValueError):
                        collector.collect(smoke, "smoke")
                    launch.assert_not_called()
                    self.assertFalse(Path(smoke.output).exists())
                    self.assertEqual(lock.read_bytes(), retained)

    def test_checkout_admission_refuses_consistently_attested_dirty_source(self):
        path = self.root / "dirty-bundle.json"
        collector.save(path, self.bundle)
        augmented = copy.deepcopy(self.identity)
        augmented["build"]["runtime_image_preflight"] = {"path": str(path), "sha256": collector.sha256(path)}
        augmented["build"]["source_status_porcelain"] = " M Cargo.lock\n"
        augmented["build"]["provenance_commands"][3]["record"]["stdout"] = " M Cargo.lock\n"
        with self.assertRaisesRegex(ValueError, "dirty source"):
            collector.runtime_closure_admission(augmented, TARGET, "normal")

    def test_missing_representative_path_cannot_reuse_complete_summary(self):
        broken = copy.deepcopy(self.bundle)
        broken["attempts"].pop()
        with self.assertRaises(ValueError):
            self.validate(broken)

    def test_each_path_must_bind_attested_executable_and_dependency_bytes(self):
        for image in (0, 1):
            with self.subTest(image=image):
                broken = copy.deepcopy(self.bundle)
                for attempt in broken["attempts"]:
                    attempt["receipt"]["images"][image]["sha256"] = "f" * 64
                self.close(broken)
                with self.assertRaises(ValueError):
                    self.validate(broken)
        broken = copy.deepcopy(self.bundle)
        attempt = broken["attempts"][-1]
        log = self.run.log().replace(self.native.load_line(7, self.native.UUID_B, str(self.run.lib)), "")
        attempt["receipt"] = self.run.discovery(log, stdout=attempt["receipt"]["record"]["stdout"])
        self.close(broken)
        with self.assertRaises(ValueError):
            self.validate(broken)

    def test_loader_environment_source_and_reserved_budget_cannot_drift(self):
        for change in ("environment", "source", "timeout", "duration"):
            with self.subTest(change=change):
                broken = copy.deepcopy(self.bundle)
                if change == "environment":
                    broken["plan"]["environment"]["DYLD_LIBRARY_PATH"] = "/other"
                elif change == "source":
                    broken["plan"]["collector"]["tools/template-matching-efficiency.py"] = "f" * 64
                elif change == "timeout":
                    broken["attempts"][-1]["campaign_elapsed_before"] = 899
                    broken["attempts"][-1]["campaign_elapsed_after"] = 900
                else:
                    broken["attempts"][-1]["receipt"]["duration_seconds"] = 216
                    self.close(broken)
                with self.assertRaises(ValueError):
                    self.validate(broken)

    def test_measurement_receipt_rejects_endpoint_only_coverage_drift_and_overrun(self):
        runtime = self.validate()
        stdout = self.bundle["attempts"][1]["receipt"]["record"]["stdout"]
        receipt = self.run.observation(runtime["closure"], stdout=stdout)
        attempt = {"record": receipt["record"],
                   "runtime_observation": {key: value for key, value in receipt.items() if key != "record"},
                   "timeout_seconds": 300, "cleanup_seconds": 5, "runtime_binding_seconds": 120}
        collector.validate_runtime_attempt(attempt, runtime)
        for change in ("endpoint_only", "coverage", "threads", "duration"):
            with self.subTest(change=change):
                broken = copy.deepcopy(attempt)
                if change == "endpoint_only":
                    del broken["runtime_observation"]
                elif change == "coverage":
                    log = self.run.log().replace(self.native.load_line(7, self.native.UUID_B, str(self.run.lib)), "")
                    partial = self.run.observation(runtime["closure"], log, stdout=stdout)
                    broken["record"] = partial.pop("record")
                    broken["runtime_observation"] = partial
                elif change == "threads":
                    value = collector.strict_json(broken["record"]["stdout"])
                    for point in ("before", "after"):
                        value["opencv_parallel_threads"][point]["threads"] += 1
                    broken["record"]["stdout"] = json.dumps(value)
                else:
                    broken["runtime_observation"]["duration_seconds"] = 546
                with self.assertRaises(ValueError):
                    collector.validate_runtime_attempt(broken, runtime)

    def test_measurement_image_paths_remain_case_sensitive_on_macos(self):
        runtime = self.validate()
        stdout = self.bundle["attempts"][1]["receipt"]["record"]["stdout"]
        receipt = self.run.observation(runtime["closure"], stdout=stdout)
        attempt = {"record": receipt["record"],
                   "runtime_observation": {key: value for key, value in receipt.items() if key != "record"},
                   "timeout_seconds": 300, "cleanup_seconds": 5, "runtime_binding_seconds": 120}
        runtime["plan"]["binary"]["path"] = runtime["plan"]["binary"]["path"].upper()
        with self.assertRaisesRegex(ValueError, "omits an attested image"):
            collector.validate_runtime_attempt(attempt, runtime)

    def test_preflight_stops_at_first_missing_or_nonzero_statistical_window(self):
        self.enable_timing()
        metadata = self.root / "original-build.json"
        collector.save(metadata, self.identity["build"])
        task = self.bundle["attempts"][1]["task"]
        value = self.bundle["attempts"][1]["receipt"]["record"]["stdout"]
        duplicate = self.native.load_line(7, self.native.UUID_B, str(self.run.lib))
        for name, markers in (("missing", ""), ("in-window", self.native.timing_row(task["workload"], inside=duplicate))):
            with self.subTest(name=name):
                destination = self.root / name
                args = collector.parser().parse_args([
                    "runtime-preflight", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
                    "--binary", str(self.run.main), "--build-metadata", str(metadata), "--output", str(destination)])
                failed = self.run.discovery(self.run.log() + markers, stdout=value)
                failed["duration_seconds"] = 0.2
                with (mock.patch.object(collector, "host_identity", return_value=self.bundle["plan"]["host"]),
                      mock.patch.object(collector, "environment", return_value=self.bundle["plan"]["environment"]),
                      mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
                      mock.patch.object(collector.macos_images, "discover", side_effect=[
                          self.bundle["attempts"][0]["receipt"], failed]) as discover):
                    result = collector.runtime_preflight(args)
                self.assertEqual(result["status"], "incomplete")
                self.assertEqual(discover.call_count, 2)
                self.assertFalse((destination / "bundle.json").exists())
                attempt = collector.load(destination / "attempts/0001.json")
                self.assertEqual(attempt["receipt"]["record"], failed["record"])
                self.assertIn("timing boundary evidence refused", attempt["provenance_error"])

    def test_interruption_survives_failed_receipt_persistence(self):
        self.enable_timing()
        args = collector.parser().parse_args([
            "runtime-preflight", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--binary", "fixture", "--build-metadata", "fixture.json", "--output", str(self.root / "out")])
        original_save = collector.save

        def save(path, value, **options):
            if Path(path).parent.name == "attempts":
                raise OSError("receipt storage failed")
            original_save(path, value, **options)

        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "binary_identity", return_value=self.identity),
              mock.patch.object(collector, "check_files"),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector.macos_images, "discover", side_effect=KeyboardInterrupt("primary")),
              mock.patch.object(collector, "save", side_effect=save),
              self.assertRaisesRegex(KeyboardInterrupt, "primary")):
            collector.runtime_preflight(args)


# Independent protocol3 input/oracle data; do not derive it from collector validators.
V3_ROWS = (
    ("session-retained-rgba-full-hit", "retained-hit", "rgba8", "full"),
    ("session-retained-rgba-full-miss", "retained-miss", "rgba8", "full"),
    ("session-retained-rgba-roi-hit", "retained-hit", "rgba8", "roi"),
    ("session-retained-rgba-roi-miss", "retained-miss", "rgba8", "roi"),
    ("session-retained-bgra-full-hit", "retained-hit", "bgra8", "full"),
    ("session-retained-bgra-full-miss", "retained-miss", "bgra8", "full"),
    ("session-retained-bgra-roi-hit", "retained-hit", "bgra8", "roi"),
    ("session-retained-bgra-roi-miss", "retained-miss", "bgra8", "roi"),
    ("session-group8-rgba-full", "group", "rgba8", "full"),
    ("session-group8-rgba-same-roi", "group", "rgba8", "roi"),
    ("session-group8-rgba-distinct-roi", "distinct-group", "rgba8", "distinct"),
    ("session-group8-bgra-full", "group", "bgra8", "full"),
    ("session-group8-bgra-same-roi", "group", "bgra8", "roi"),
    ("session-group8-bgra-distinct-roi", "distinct-group", "bgra8", "distinct"),
    ("session-owner8-rgba-roi", "owner-group", "rgba8", "roi"),
    ("session-owner8-bgra-roi", "owner-group", "bgra8", "roi"),
    ("session-patch64-rgba-full", "patch64", "rgba8", "full"),
    ("session-patch64-rgba-roi", "patch64", "rgba8", "roi"),
    ("session-patch64-bgra-full", "patch64", "bgra8", "full"),
    ("session-patch64-bgra-roi", "patch64", "bgra8", "roi"),
    ("session-turnover-rgba-full", "turnover", "rgba8", "full"),
    ("session-turnover-rgba-roi", "turnover", "rgba8", "roi"),
    ("session-turnover-bgra-full", "turnover", "bgra8", "full"),
    ("session-turnover-bgra-roi", "turnover", "bgra8", "roi"),
    ("query-match-rgba-full", "query-match", "rgba8", "full"),
    ("query-match-rgba-roi", "query-match", "rgba8", "roi"),
    ("query-match-bgra-full", "query-match", "bgra8", "full"),
    ("query-match-bgra-roi", "query-match", "bgra8", "roi"),
    ("query-deadline-rgba-full", "query-deadline", "rgba8", "full"),
    ("query-deadline-rgba-roi", "query-deadline", "rgba8", "roi"),
    ("query-deadline-bgra-full", "query-deadline", "bgra8", "full"),
    ("query-deadline-bgra-roi", "query-deadline", "bgra8", "roi"),
    ("lifecycle-session-rgba-roi", "lifecycle-session", "rgba8", "roi"),
    ("lifecycle-session-bgra-roi", "lifecycle-session", "bgra8", "roi"),
    ("lifecycle-independent-rgba-4k", "lifecycle-independent", "rgba8", "4k"),
    ("lifecycle-independent-bgra-4k", "lifecycle-independent", "bgra8", "4k"),
)
V3_DISTINCT_REGIONS = (
    (64, 48, 384, 228), (136, 48, 456, 228), (208, 48, 528, 228), (280, 48, 600, 228),
    (64, 120, 384, 300), (136, 120, 456, 300), (208, 120, 528, 300), (280, 120, 600, 300),
)
V3_CASES = {
    # Sequence, unit, successful backend calls.
    "retained-hit": (["H0"], "single-search", 1),
    "retained-miss": (["M"], "single-search", 1),
    "group": (["H0"], "group8", 8),
    "distinct-group": (["H0"], "group8", 8),
    "owner-group": (["H0"], "group8", 8),
    "patch64": (["H64"], "single-search", 1),
    "turnover": (["M", "H0", "M", "H1"], "turnover-cycle", 4),
    "query-match": (["M", "H0", "H1"], "query-match", 3),
    "query-deadline": (["M"], "query-deadline", 1),
    "lifecycle-session": (["H0"], "lifecycle", 1),
    "lifecycle-independent": (["H0"], "lifecycle", 4),
}
V3_APPARATUS_SOURCES = (
    "crates/mado-pilot/benches/template-matching-efficiency.rs",
    "crates/mado-pilot/benches/support/template_efficiency_fixtures.rs",
    "crates/mado-pilot/benches/support/template_efficiency_session_fixtures.rs",
    "crates/mado-pilot/benches/support/template_efficiency_sessions.rs",
    "crates/mado-pilot/benches/support/template_efficiency_queries.rs",
    "crates/mado-pilot/benches/support/template_efficiency_formats.rs",
    "crates/mado-pilot/benches/support/template_efficiency_watch.rs",
    "crates/mado-pilot/benches/support/template_efficiency_runtime.rs",
    "crates/mado-pilot/benches/support/process_cpu.rs",
    "crates/mado-pilot/benches/support/ocr_text_watch_resources.rs",
    "crates/support/testkit/src/match_fixtures.rs",
)
V3_OWNER_ORDER = ("before-setup", "after-warmups", "after-samples", "frame-and-result-released",
                  "session-closed", "runtime-and-source-released", "final-backend-released")
V3_SESSION_OWNER_ORDER = ("before-setup", "live-outcome", "caller-frame-released", "session-closed",
                          "mapping-released", "outcome-released", "runtime-and-source-released",
                          "final-backend-released")
V3_INDEPENDENT_OWNER_ORDER = ("before-setup", "two-backends-live", "a-released", "b-reused",
                              "frame-and-result-released", "final-backend-released")


def v3_fixture_contract(case, fmt):
    sequence, unit, calls = V3_CASES[case]
    group = case in ("group", "distinct-group", "owner-group")
    return {"version": 1, "case": case, "source_format": fmt,
            "patch_extent": [64, 48] if case == "patch64" else [12, 10],
            "frame_sequence": list(sequence), "template_order": list(range(8)) if group else [0],
            "unit": unit, "searches_per_unit": calls,
            "sample_strata": {"hit": 2, "miss": 2} if case == "turnover" else {},
            "max_independent_backends": 2 if case == "lifecycle-independent" else 1}


def v3_fixture_regions(case, region):
    if region == "distinct":
        return [list(bounds) for bounds in V3_DISTINCT_REGIONS]
    bounds = {"full": [0, 0, 1920, 1080], "roi": [64, 48, 384, 228], "4k": [0, 0, 3840, 2160]}[region]
    return [list(bounds) for _ in range(8 if case in ("group", "owner-group") else 1)]


def v3_fixture_work(row, index):
    case = row["session_contract"]["case"]
    left, top, right, bottom = row["effective_regions"][0]
    roi = right - left == 320
    calls, conversions, copies = {
        "retained-hit": (1, int(index == 0), int(roi)),
        "retained-miss": (1, int(index == 0), int(roi)),
        "patch64": (1, int(index == 0), int(roi)),
        "group": (8, 1, 8 if roi else 0),
        "distinct-group": (8, 8, 8),
        "owner-group": (8, 1, 1),
        "turnover": (4, 4, 4 if roi else 0),
        "query-match": (3, 3, 3 if roi else 0),
        "query-deadline": (1, 1, int(roi)),
        "lifecycle-session": (1, 1, 1),
        "lifecycle-independent": (4, 2, 0),
    }[case]
    pixels = (right - left) * (bottom - top)
    return {"accounting_contract": "prepared-cache-row-winner-v1",
            "backend_find_calls": calls, "backend_find_completions": calls, "backend_find_failures": 0,
            "bgr_conversions": conversions, "bgr_converted_bytes": conversions * pixels * 3,
            "mapping_copies": copies, "mapping_copied_bytes": copies * pixels * 4, "correct": True}


def v3_manifest(build_mode="normal", role="baseline"):
    value = v2_manifest(build_mode)
    value["format_version"] = 3
    small = [{"id": f"efficiency.patch{i}", "sha256": f"{100 + i:064x}"} for i in range(8)]
    large = [{"id": f"efficiency.patch64x48.{i}", "sha256": f"{200 + i:064x}"} for i in range(8)]
    scenes = [
        {"name": name, "width": width, "height": height, "format": fmt, "stride": width * 4,
         "patch_extent": extent, "sha256": f"{300 + index:064x}"}
        for index, (name, width, height, extent, fmt) in enumerate(
            (name, width, height, extent, fmt) for fmt in ("rgba8", "bgra8")
            for name, width, height, extent in (
                ("H0", 1920, 1080, [12, 10]), ("H1", 1920, 1080, [12, 10]),
                ("M", 1920, 1080, [12, 10]), ("H64", 1920, 1080, [64, 48]),
                ("H0", 3840, 2160, [12, 10])))]
    legacy_scenes = [
        {"width": width, "height": height, "variant": variant, "format": fmt, "sha256": f"{checksum:064x}"}
        for width, height, variant, fmt, checksum in (
            (96, 64, 0, "Rgba8", 400), (1920, 1080, 0, "Rgba8", 300),
            (3840, 2160, 0, "Rgba8", 304), (1920, 1080, 1, "Rgba8", 301),
            (1920, 1080, 2, "Rgba8", 401), (4096, 2732, 0, "Rgba8", 402),
            (1920, 1080, 0, "Bgra8", 305))]
    value["fixture"] = {
        "license": "repository-first-party", "generator": "template-efficiency-integer-v3-session",
        "historical_component_generator": "template-efficiency-integer-v2-direct-formats",
        "background_seed": 0x31415926, "patch_seed_base": 0xABC00000, "patch_width": 12, "patch_height": 10,
        "roi": [64, 48, 384, 228], "score_tolerance": 1e-5,
        "templates": small, "scenes": legacy_scenes,
        "score_maps": [{"id": row["id"], "sha256": f"{500 + i:064x}"}
                       for i, row in enumerate(collector.DEFINITIONS.values()) if row["category"] == "selection"],
        "direct_formats": collector.direct_format_inputs(),
        "watcher": {**{key: f"{600 + i:064x}" for i, key in enumerate((
            "scene_rgba_sha256", "background_rgba_sha256", "patch_png_sha256", "absent_png_sha256"))},
            "roi": [16, 8, 48, 32], "min_score": 0.95, "consecutive_frames": 2},
    }
    value["apparatus"] = {
        "accounting_contract": "prepared-cache-row-winner-v1",
        "source_files": {path: ("3" if role == "baseline" else "4") * 64
                         for path in V3_APPARATUS_SOURCES}}
    cache = {
        "retained-hit": "persistent-frame-and-backend", "retained-miss": "persistent-frame-and-backend",
        "patch64": "persistent-frame-and-backend", "group": "cold-engine-backend-and-frame",
        "distinct-group": "cold-engine-backend-and-frame", "owner-group": "cold-with-live-compatible-roi-owner",
        "turnover": "persistent-backend-new-frames", "query-match": "persistent-backend-fresh-query-session",
        "query-deadline": "persistent-backend-fresh-query-session", "lifecycle-session": "fresh-owner-lifecycle",
        "lifecycle-independent": "two-independent-backends-one-frame"}
    timing = {
        "single-search": "public Session search through bounded scalar projection and outcome release; setup and teardown separate",
        "group8": "eight public Session searches, bounded projections and outcome releases; setup, compatible mapping owner and teardown separate",
        "turnover-cycle": "publication/acquisition through four searches and final release per cycle; overlapping hit/miss search intervals retained separately",
        "query": "producer release through actual query wait, terminal projection and producer join; session close, retained-terminal checks and query/outcome release separate",
        "lifecycle-session": "complete public Session owner script including retained outcome after close and final backend release; contract only",
        "lifecycle-independent": "complete real Matcher owner script over one retained frame and two independent backends; contract only"}
    rows = []
    for workload, case, fmt, region in V3_ROWS:
        statistical = case not in ("query-deadline", "lifecycle-session", "lifecycle-independent")
        definition = {"id": workload, "category": "session" if statistical else "contract",
                      "improvement": False, "statistical": statistical}
        contract = v3_fixture_contract(case, fmt)
        independent = contract["case"] == "lifecycle-independent"
        boundary = ("query" if contract["case"].startswith("query-") else
                    contract["case"] if contract["case"].startswith("lifecycle-") else contract["unit"])
        rows.append({
            **definition, "cache_state": cache[contract["case"]], "timing_boundary": timing[boundary],
            "width": 3840 if independent else 1920, "height": 2160 if independent else 1080,
            "result_limit": 8, "suppression": "DropOverlapping", "min_score": 0.9,
            "warmups": 20 if definition["statistical"] else 1,
            "sample_count": 200 if definition["statistical"] else 3,
            "effective_regions": v3_fixture_regions(case, region),
            "change_policy": "AnalysisAlways" if contract["case"].startswith("query-") else None,
            "format_contract": None, "session_contract": contract})
    value["workloads"].extend(rows)
    value["fixture"]["sessions"] = {
        "generator": "template-efficiency-session-v1", "license": "repository-first-party",
        "min_score": 0.9, "result_limit": 8, "suppression": "DropOverlapping", "score_tolerance": 1e-5,
        "small_templates": small, "large_templates": large, "scenes": scenes,
        "origins": [[80 + i % 4 * 72, 64 + i // 4 * 72] for i in range(8)],
        "distinct_regions": [list(bounds) for bounds in V3_DISTINCT_REGIONS],
        "workloads": copy.deepcopy(rows)}
    return value


def v3_checkpoints(order, build_mode, held=None):
    normal = build_mode == "normal"
    held = held if held is not None else [0] * len(order)
    return [{"name": name, "rss_bytes": 12 * collector.MIB, "peak_rss_bytes": 16 * collector.MIB,
             "private_bytes": None, "footprint_bytes": 12 * collector.MIB,
             "heap_bytes": None if normal else 6 * collector.MIB,
             "bgr_retained_bytes": None if normal else retained,
             "bgr_evicted_bytes": None if normal else 0, "owner_correct": None if normal else True}
            for name, retained in zip(order, held)]


def v3_unit(row, index, build_mode):
    contract = row["session_contract"]
    case = contract["case"]
    normal = build_mode == "normal"
    engine = 1 if case in ("retained-hit", "retained-miss", "patch64", "turnover") else index + 1

    def stamp(sequence):
        return [f"engine#{engine}", 1, 0, sequence, 0]

    count = 1 if case == "query-match" else 0 if case == "query-deadline" else contract["searches_per_unit"]
    projections = []
    for position in range(count):
        template = position if case in ("group", "distinct-group", "owner-group") else 0
        left, top = 80 + template % 4 * 72, 64 + template // 4 * 72
        if case == "query-match" or case == "turnover" and position == 3:
            left, top = left + 3, top + 2
        width, height = contract["patch_extent"]
        absent = case == "retained-miss" or case == "turnover" and position in (0, 2)
        sequence = index * 4 + position if case == "turnover" else 2 if case == "query-match" else 0
        projections.append({
            "stamp": stamp(sequence), "envelope_correct": True, "best_consistent": True,
            "matches": [] if absent else [{"bounds": [left, top, left + width, top + height],
                                          "score": 1.0, "template_matches": True}]})
    query = None
    if case.startswith("query-"):
        matched = case == "query-match"
        query = {"terminal": "matched" if matched else "deadline-exceeded",
                 "completed": [{"stamp": stamp(position), "admitted": position + 1,
                                "completed": position + 1, "confirmed": position,
                                "pending": 0, "in_flight": 0, "exceptional": 0}
                               for position in range(2 if matched else 1)],
                 "terminal_stamp": stamp(2) if matched else None,
                 "confirmed": 2 if matched else 0, "after_close_same": True}
    setup = {"elapsed_ns": 50, "cpu_ns": 20, "included_in_primary": case.startswith("lifecycle-")}
    teardown = dict(setup)
    if case == "owner-group":
        setup["mapping_created"] = True
        teardown["mapping_released"] = True
    if case.startswith("query-"):
        setup.update(session_opened=True, query_started=True, producer_ready=True)
        teardown.update(session_closed=True, retained_terminal_checked=True,
                        query_and_outcome_released=True, runtime_and_source_released=True)
    lifecycle, checkpoints = None, []
    if case == "lifecycle-session":
        lifecycle = {"retained_after_close": True, "closed_search_refused": True,
                     "mapping_valid_after_close": True, "source_released": True}
        checkpoints = v3_checkpoints(V3_SESSION_OWNER_ORDER, build_mode,
                                     [0, 172800, 172800, 172800, 172800, 172800, 172800, 0])
    if case == "lifecycle-independent":
        lifecycle = {key: None if normal else True for key in ("a_clone_reused", "b_independent", "b_reused")}
        lifecycle["source_released"] = True
        checkpoints = v3_checkpoints(V3_INDEPENDENT_OWNER_ORDER, build_mode,
                                     [0, 49_766_400, 24_883_200, 24_883_200, 24_883_200, 0])
    return {"correct": True, "error": None, "projections": projections, "query": query,
            "strata": {name: [{"elapsed_ns": 100 + position, "cpu_ns": 40 + position} for position in range(2)]
                       for name in contract["sample_strata"]},
            "setup": setup, "teardown": teardown, "lifecycle": lifecycle, "checkpoints": checkpoints,
            "work": None if normal else v3_fixture_work(row, index)}


def v3_payload(workload, build_mode="normal", *, role="baseline", candidate=False, smoke=False):
    declaration = v3_manifest(build_mode, role)
    value = v2_payload(workload if workload in collector.DEFINITIONS else "single-small96x64",
                       build_mode, candidate=candidate, smoke=smoke)
    value.update(format_version=3, fixture=declaration["fixture"], apparatus=declaration["apparatus"])
    if workload not in collector.SESSION_DEFINITIONS:
        return value
    row = copy.deepcopy(next(row for row in declaration["workloads"] if row["id"] == workload))
    if smoke:
        row.update(warmups=1, sample_count=3)
    warmups, count = row["warmups"], row["sample_count"]
    base = value["workloads"][0]
    elapsed, cpu = (500_000, 250_000) if candidate else (1_000_000, 500_000)
    row.update(warmup_samples=[{"elapsed_ns": elapsed, "cpu_ns": cpu, "correct": True} for _ in range(warmups)],
               samples=[{"elapsed_ns": elapsed, "cpu_ns": cpu, "correct": True} for _ in range(count)],
               metrics=base["metrics"], oracle_failures=0)
    row["metrics"].update(iterations=warmups + count, cpu_ns=count * cpu)
    row["unit_observations"] = [v3_unit(row, index, build_mode) for index in range(warmups + count)]
    row["owner_observations"] = v3_checkpoints(V3_OWNER_ORDER, build_mode)
    if build_mode == "instrumented":
        for name in ("backend_find_calls", "backend_find_failures", "bgr_conversions", "bgr_converted_bytes",
                     "mapping_copies", "mapping_copied_bytes"):
            row["metrics"][name] = sum(unit["work"][name] for unit in row["unit_observations"])
        row["metrics"].update(score_copied_bytes=0, bgr_retained_bytes=0, bgr_evicted_bytes=0,
                              bgr_evicted_peak_bytes=0, bgr_retained_peak_bytes=(
                                  49_766_400 if row["session_contract"]["case"] == "lifecycle-independent" else 172800))
    value["workloads"] = [row]
    return value


def v3_identity(root, build_mode="normal", role="baseline"):
    binding = v2_identity(root, build_mode, role)
    declaration = v3_manifest(build_mode, role)
    build = binding["build"]
    build.update(protocol_version=3, apparatus=copy.deepcopy(declaration["apparatus"]),
                 benchmark_manifest=collector.manifest_identity(declaration, TARGET, "4.12.0", 3, build_mode))
    build["provenance_commands"][0] = {
        "argv": [binding["path"], "--protocol-version", "3", "--list"], "record": process_record(declaration)}
    return binding


def v3_plan(root, mode, build_mode="normal"):
    value = v2_plan(root, mode, build_mode)
    rows = list(collector.definitions_for(build_mode, 3))
    binaries = {"baseline": v3_identity(root, build_mode)}
    if mode == "control":
        binaries["control"] = copy.deepcopy(binaries["baseline"])
    if mode == "comparison":
        binaries["candidate"] = v3_identity(root, build_mode, "candidate")
    jobs = collector.schedule(mode, rows, 3)
    value.update(format_version=3, binaries=binaries, workloads=rows, schedule=jobs,
                 policy=copy.deepcopy(collector.V3_POLICY),
                 execution_limits=collector.execution_limits(mode, len(jobs)))
    return value


def v3_parsed_evidence(root, mode, build_mode="normal", workload=None):
    specification = v3_plan(root, mode, build_mode)
    if workload is not None:
        specification["workloads"] = [workload]
        specification["schedule"] = collector.schedule(mode, [workload], 3)
        specification["execution_limits"] = collector.execution_limits(mode, len(specification["schedule"]))
    frozen = collector.manifest_identity(v3_manifest(build_mode), TARGET, "4.12.0", 3, build_mode)
    cohorts = {}
    for role in specification["binaries"]:
        cohorts[role] = {}
        for workload in specification["workloads"]:
            summary = collector.observations(
                v3_payload(workload, build_mode, role="candidate" if role == "candidate" else "baseline",
                           candidate=role == "candidate"), frozen if role != "candidate" else
                specification["binaries"]["candidate"]["build"]["benchmark_manifest"], workload, False)
            cohorts[role][workload] = [
                {**summary, "process": process,
                 "position": (process % 2 if role == "baseline" else 1 - process % 2)
                             if mode in ("control", "comparison") else 0} for process in range(6)]
    token = {"calibration": "1", "control": "2", "comparison": "3"}[mode]
    if mode == "control":
        specification["calibration"] = {
            "path": str(root / "calibration"), "plan_sha256": "1" * 64,
            "attempts": [{"file": "0000.json", "sha256": "1" * 64}]}
    return {"plan": specification, "plan_sha256": token * 64,
            "attempts": [{"file": "0000.json", "sha256": token * 64}],
            "manifest": frozen, "observations": cohorts, "issues": [],
            "native_threads": {role: collector.effective_threads(v3_manifest(build_mode))
                               for role in specification["binaries"]}}


def persist_v3_smoke(root, workload, build_mode="normal"):
    root.mkdir()
    (root / "started").mkdir()
    (root / "attempts").mkdir()
    specification = v3_plan(root.parent, "smoke", build_mode)
    specification.update(workloads=[workload], schedule=collector.schedule("smoke", [workload], 3),
                         method_review=None)
    specification["execution_limits"] = collector.execution_limits("smoke", len(specification["schedule"]))
    collector.save(root / "plan.json", specification)
    for index, task in enumerate(specification["schedule"]):
        binary = specification["binaries"][task["role"]]
        attempt = {"task": task, "argv": collector.command(binary, task, "smoke", 3),
                   "plan_sha256": collector.sha256(root / "plan.json"), "started_unix_ns": index + 1,
                   "campaign_elapsed_before": index, "output_limit_bytes": 8 * collector.MIB,
                   "cleanup_seconds": 5, "runtime_binding_seconds": 0}
        collector.save(root / "started" / f"{task['id']}.json", attempt)
        value = v3_manifest(build_mode) if task["workload"] is None else v3_payload(workload, build_mode, smoke=True)
        attempt.update(identity_before=binary, identity_after=binary, identity_error=None,
                       campaign_elapsed_after=index + 0.1, record=process_record(value), timeout_seconds=90)
        collector.save(root / "attempts" / f"{task['id']}.json", attempt)
    return specification


class ProtocolThreeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def parse(self, value, workload, build_mode="normal", smoke=True):
        frozen = collector.manifest_identity(v3_manifest(build_mode), TARGET, "4.12.0", 3, build_mode)
        return collector.observations(value, frozen, workload, smoke)

    def test_protocol_three_preserves_semantic_corpora(self):
        for build_mode, count in (("normal", 54), ("instrumented", 57)):
            frozen = collector.manifest_identity(v3_manifest(build_mode), TARGET, "4.12.0", 3, build_mode)
            self.assertEqual(len(frozen["workloads"]), count)
            self.assertEqual([row["id"] for row in frozen["workloads"]][-36:],
                             [row[0] for row in V3_ROWS])
            self.assertEqual(sum(row["statistical"] for row in frozen["workloads"]), 41)
            self.assertTrue(all(not row["improvement"] for row in frozen["workloads"][-36:]))
        self.assertEqual(len(collector.manifest_identity(manifest(), TARGET, "4.12.0")["workloads"]), 21)
        self.assertEqual(len(collector.manifest_identity(v2_manifest(), TARGET, "4.12.0", 2, "normal")["workloads"]), 18)
        self.assertNotEqual(collector.V3_POLICY["id"], collector.V2_POLICY["id"])

    def test_every_new_row_validates_in_both_modes(self):
        for mode in ("normal", "instrumented"):
            for workload, _, _, _ in V3_ROWS:
                with self.subTest(mode=mode, workload=workload):
                    summary = self.parse(v3_payload(workload, mode, smoke=True), workload, mode)
                    self.assertEqual(summary["unavailable"], [])
                    self.assertEqual(len(summary["unit_observations"]), 4)
                    self.assertEqual(len(summary["owner_observations"]), 7)

    def test_row_order_options_sequence_and_region_are_fixed(self):
        for change in ("order", "sequence", "searches", "strata", "options", "regions"):
            value = v3_manifest()
            row = next(row for row in value["workloads"] if row["id"] == "session-turnover-rgba-roi")
            if change == "order":
                value["workloads"][-1], value["workloads"][-2] = value["workloads"][-2], value["workloads"][-1]
            elif change == "sequence":
                row["session_contract"]["frame_sequence"] = ["H0", "M", "M", "H1"]
            elif change == "searches":
                row["session_contract"]["searches_per_unit"] = True
            elif change == "strata":
                row["session_contract"]["sample_strata"] = {}
            elif change == "options":
                row["min_score"] = 0.99
            else:
                row["effective_regions"] = [[0, 0, 1920, 1080]]
            with self.subTest(change=change), self.assertRaises(ValueError):
                collector.manifest_identity(value, TARGET, "4.12.0", 3, "normal")

    def test_role_apparatus_is_separate_but_mandatory_and_unchanged(self):
        base = collector.manifest_identity(v3_manifest(), TARGET, "4.12.0", 3, "normal")
        candidate = collector.manifest_identity(v3_manifest(role="candidate"), TARGET, "4.12.0", 3, "normal")
        self.assertNotEqual(base, candidate)
        self.assertEqual(collector.common_manifest(base), collector.common_manifest(candidate))
        workload = "session-retained-rgba-roi-hit"
        changed = v3_payload(workload, smoke=True)
        changed["apparatus"] = candidate["apparatus"]
        with self.assertRaises(ValueError):
            self.parse(changed, workload)
        for change in ("missing", "source", "accounting", "semantic-hash", "common-input"):
            value = v3_manifest()
            if change == "missing":
                del value["apparatus"]
            elif change == "source":
                value["apparatus"]["source_files"].pop(next(iter(value["apparatus"]["source_files"])))
            elif change == "accounting":
                value["apparatus"]["accounting_contract"] = "unreviewed-historical-port"
            elif change == "semantic-hash":
                value["fixture"]["sessions"]["helper_source_sha256"] = "5" * 64
            else:
                value["fixture"]["sessions"]["scenes"][2]["sha256"] = "5" * 64
                other = collector.manifest_identity(value, TARGET, "4.12.0", 3, "normal")
                self.assertNotEqual(collector.common_manifest(base), collector.common_manifest(other))
                continue
            with self.subTest(change=change), self.assertRaises(ValueError):
                collector.manifest_identity(value, TARGET, "4.12.0", 3, "normal")

    def test_attestation_rejects_mixed_role_source_or_missing_apparatus(self):
        for change in ("metadata", "manifest", "missing"):
            binding = v3_identity(self.root)
            if change == "metadata":
                binding["build"]["apparatus"]["source_files"][collector.APPARATUS_SOURCES[0]] = "6" * 64
            elif change == "manifest":
                binding["build"]["benchmark_manifest"]["apparatus"]["source_files"][collector.APPARATUS_SOURCES[0]] = "6" * 64
            else:
                del binding["build"]["apparatus"]
            with self.subTest(change=change), self.assertRaises(ValueError):
                collector.validate_v2_build(binding, TARGET, "normal", 3)

    def test_current_attest_binds_all_apparatus_bytes_and_retains_failures(self):
        for drift in (False, True):
            with self.subTest(drift=drift):
                root = self.root / ("drift" if drift else "bound")
                root.mkdir()
                binary = root / "binary"
                native = root / "libopencv_core.fixture"
                metadata = root / "build.json"
                binary.write_bytes(b"explicit prebuilt executable")
                native.write_bytes(b"native fixture")
                (root / "Cargo.lock").write_bytes(b"fixture lock")
                declaration = v3_manifest()
                declaration["target"] = NATIVE_TARGET
                declaration["loaded_native_runtime"] = {
                    "method": "K32EnumProcessModules-GetModuleFileNameW" if os.name == "nt" else RUNTIME["method"],
                    "images": [{"path": str(native), "sha256": collector.sha256(native)}]}
                for index, path in enumerate(collector.APPARATUS_SOURCES):
                    source = root / path
                    source.parent.mkdir(parents=True, exist_ok=True)
                    source.write_bytes(f"authored apparatus fixture {index}\n".encode())
                    declaration["apparatus"]["source_files"][path] = collector.sha256(source)
                if drift:
                    declaration["apparatus"]["source_files"][collector.APPARATUS_SOURCES[-1]] = "0" * 64
                records = [process_record(declaration), process_record("a" * 40 + "\n"),
                           process_record("c" * 40 + "\n"), process_record(""), process_record("rustc fixture\n")]
                args = collector.parser().parse_args([
                    "attest", "--protocol-version", "3", "--build-mode", "normal", "--target", NATIVE_TARGET, "--binary", str(binary),
                    "--build-argv-json", '["cargo","bench","--no-run","--locked"]', "--output", str(metadata)])
                with (mock.patch.object(collector, "ROOT", root),
                      mock.patch.object(collector, "host_identity", return_value={"target": NATIVE_TARGET}),
                      mock.patch.object(collector.shutil, "which", return_value=sys.executable),
                      mock.patch.object(collector, "run_process", side_effect=records) as launch):
                    if drift:
                        with self.assertRaises(ValueError):
                            collector.attest(args)
                    else:
                        collector.attest(args)
                self.assertEqual(launch.call_count, 1 if drift else 5)
                probe = collector.load(str(metadata) + ".probe.json")
                self.assertEqual(probe["format_version"], 3)
                self.assertEqual(len(probe["commands"]), launch.call_count)
                self.assertEqual(metadata.exists(), not drift)
                if not drift:
                    build = collector.load(metadata)
                    self.assertEqual(build["apparatus"], declaration["apparatus"])
                    binding = {"path": str(binary), "build": build}
                    collector.validate_v2_build(binding, NATIVE_TARGET, "normal", 3)
                    with self.assertRaises(ValueError):
                        collector.runtime_closure_admission(binding, NATIVE_TARGET, "normal")

    def test_error_bearing_units_and_partial_checkpoints_remain_failed(self):
        workload = "lifecycle-session-rgba-roi"
        for mode in ("normal", "instrumented"):
            for location in ("unit", "unit-checkpoint", "final-checkpoint"):
                value = v3_payload(workload, mode, smoke=True)
                row = value["workloads"][0]
                if location == "unit":
                    row["unit_observations"][0]["error"] = "fixture teardown failure"
                elif location == "unit-checkpoint":
                    row["unit_observations"][0]["checkpoints"][3]["error"] = "fixture close failure"
                else:
                    row["owner_observations"][-1]["error"] = "fixture release failure"
                with self.subTest(mode=mode, location=location), self.assertRaises(ValueError):
                    self.parse(value, workload, mode)

    def test_missing_scene_template_or_fixture_rows_cannot_be_attested(self):
        for key in ("small_templates", "large_templates", "scenes", "workloads"):
            declaration = v3_manifest()
            declaration["fixture"]["sessions"][key].pop()
            with self.subTest(key=key), self.assertRaises(ValueError):
                collector.manifest_identity(declaration, TARGET, "4.12.0", 3, "normal")

    def test_missing_failed_or_reordered_units_and_final_owners_are_rejected(self):
        workload = "session-retained-rgba-roi-hit"
        for change in ("units", "owner", "failed", "owner-order", "owner-null", "final-payload", "normal-zero"):
            mode = "instrumented" if change in ("owner-null", "final-payload") else "normal"
            value = v3_payload(workload, mode, smoke=True)
            row = value["workloads"][0]
            if change == "units":
                row["unit_observations"].pop()
            elif change == "owner":
                del row["owner_observations"]
            elif change == "failed":
                row["unit_observations"][-1]["correct"] = False
            elif change == "owner-order":
                row["owner_observations"].reverse()
            elif change == "owner-null":
                row["owner_observations"][-1]["owner_correct"] = None
            elif change == "final-payload":
                row["owner_observations"][-1]["bgr_retained_bytes"] = 1
            else:
                row["owner_observations"][0]["heap_bytes"] = 0
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.parse(value, workload, mode)

    def test_public_match_count_bounds_identity_finiteness_and_best_are_checked(self):
        workload = "session-group8-bgra-distinct-roi"
        for change in ("count", "bounds", "template", "nan", "infinite", "negative", "boolean",
                       "huge-integer", "huge-negative-integer", "best", "stamp"):
            value = v3_payload(workload, smoke=True)
            projection = value["workloads"][0]["unit_observations"][0]["projections"][0]
            if change == "count":
                projection["matches"] *= 9
            elif change == "bounds":
                projection["matches"][0]["bounds"][0] += 1
            elif change == "template":
                projection["matches"][0]["template_matches"] = False
            elif change in ("nan", "infinite", "negative", "boolean", "huge-integer", "huge-negative-integer"):
                projection["matches"][0]["score"] = {"nan": float("nan"), "infinite": float("inf"),
                                                    "negative": -1, "boolean": True,
                                                    "huge-integer": 10**400, "huge-negative-integer": -(10**400)}[change]
            elif change == "best":
                projection["best_consistent"] = False
            else:
                projection["stamp"][3] = True
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.parse(value, workload)
        miss = "session-retained-rgba-full-miss"
        value = v3_payload(miss, smoke=True)
        self.assertEqual(self.parse(value, miss)["unit_observations"][0]["projections"][0]["matches"], [])
        value["workloads"][0]["unit_observations"][0]["projections"][0]["score"] = 0.1
        with self.assertRaises(ValueError):
            self.parse(value, miss)

    def test_retained_group_turnover_and_query_frame_lifetimes_remain_distinct(self):
        for workload in ("session-retained-rgba-full-hit", "session-group8-rgba-full",
                         "session-turnover-rgba-full", "query-match-rgba-full"):
            value = v3_payload(workload, smoke=True)
            units = value["workloads"][0]["unit_observations"]
            if workload.startswith("session-retained"):
                units[1]["projections"][0]["stamp"][3] += 1
            elif workload.startswith("session-group"):
                units[0]["projections"][1]["stamp"][3] += 1
            elif workload.startswith("session-turnover"):
                units[0]["projections"][1]["stamp"][3] = units[0]["projections"][0]["stamp"][3]
            else:
                for projection in units[1]["projections"]:
                    projection["stamp"][0] = "engine#1"
                for receipt in units[1]["query"]["completed"]:
                    receipt["stamp"][0] = "engine#1"
                units[1]["query"]["terminal_stamp"][0] = "engine#1"
            with self.subTest(workload=workload), self.assertRaises(ValueError):
                self.parse(value, workload)

    def test_query_receipts_terminal_close_and_excluded_teardown_are_required(self):
        for case in ("match", "deadline"):
            workload = f"query-{case}-bgra-roi"
            for change in ("receipt", "work", "sequence", "terminal", "closed", "boundary", "released", "setup"):
                value = v3_payload(workload, smoke=True)
                unit = value["workloads"][0]["unit_observations"][0]
                query = unit["query"]
                if change == "receipt":
                    query["completed"].pop()
                elif change == "work":
                    query["completed"][0]["completed"] = 0
                elif change == "sequence":
                    query["completed"][0]["stamp"][4] += 1
                    if case == "deadline":
                        query["terminal_stamp"] = ["engine#1", 1, 1, 1, 1]
                elif change == "terminal":
                    query["terminal"] = "caller-timeout"
                elif change == "closed":
                    query["after_close_same"] = False
                elif change == "boundary":
                    unit["teardown"]["included_in_primary"] = True
                elif change == "released":
                    unit["teardown"]["query_and_outcome_released"] = False
                else:
                    unit["setup"]["producer_ready"] = False
                with self.subTest(case=case, change=change), self.assertRaises(ValueError):
                    self.parse(value, workload)

    def test_declared_strata_keep_cycle_membership_and_process_local_cpu(self):
        workload = "session-turnover-rgba-roi"
        summary = self.parse(v3_payload(workload, smoke=True), workload)
        self.assertEqual(summary["strata"]["hit"]["observation_count"], 6)
        self.assertEqual(summary["strata"]["hit"]["cpu_ns"], 243)
        self.assertEqual(summary["strata"]["hit"]["p50_ns"], 100)
        self.assertEqual(summary["strata"]["hit"]["p95_ns"], 101)
        self.assertEqual(summary["cpu_ns"], 1_500_000)
        for change in ("missing", "empty", "wrong-name", "extra", "negative", "boolean", "missing-cpu"):
            value = v3_payload(workload, smoke=True)
            strata = value["workloads"][0]["unit_observations"][2]["strata"]
            if change == "missing":
                del strata["hit"]
            elif change == "empty":
                strata["hit"] = []
            elif change == "wrong-name":
                strata["hits"] = strata.pop("hit")
            elif change == "extra":
                strata["hit"].append(dict(strata["hit"][0]))
            elif change == "negative":
                strata["miss"][0]["elapsed_ns"] = -1
            elif change == "boolean":
                strata["miss"][0]["cpu_ns"] = True
            else:
                del strata["miss"][0]["cpu_ns"]
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.parse(value, workload)

    def test_missing_stratum_cpu_is_explicit_and_cannot_be_summarized_as_zero(self):
        workload = "session-turnover-bgra-full"
        value = v3_payload(workload, smoke=True)
        value["workloads"][0]["unit_observations"][-1]["strata"]["hit"][0]["cpu_ns"] = None
        value["workloads"][0]["metrics"]["cpu_reason"] = "fixture CPU unavailable"
        summary = self.parse(value, workload)
        self.assertIsNone(summary["strata.hit.cpu_ns"])
        self.assertTrue(summary["unavailable"])
        self.assertEqual(summary["strata.miss.cpu_ns"], 243)

    def test_independent_backends_have_exact_stages_and_only_row_scoped_aggregate_bound(self):
        workload = "lifecycle-independent-rgba-4k"
        value = v3_payload(workload, "instrumented", smoke=True)
        self.assertEqual(self.parse(value, workload, "instrumented")["metrics"]["bgr_retained_peak_bytes"], 49_766_400)
        for change in ("stage", "bound", "peak-below-live", "release", "reuse"):
            changed = copy.deepcopy(value)
            row = changed["workloads"][0]
            if change == "stage":
                row["unit_observations"][0]["checkpoints"][1]["bgr_retained_bytes"] = 24_883_200
            elif change == "bound":
                row["metrics"]["bgr_retained_peak_bytes"] = 64 * collector.MIB + 1
            elif change == "peak-below-live":
                row["metrics"]["bgr_retained_peak_bytes"] = 1024
            elif change == "release":
                row["unit_observations"][0]["checkpoints"][-1]["bgr_retained_bytes"] = 1
            else:
                row["unit_observations"][0]["lifecycle"]["b_reused"] = False
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.parse(changed, workload, "instrumented")
        other = "session-retained-rgba-full-hit"
        changed = v3_payload(other, "instrumented", smoke=True)
        changed["workloads"][0]["metrics"]["bgr_retained_peak_bytes"] = 32 * collector.MIB + 1
        with self.assertRaises(ValueError):
            self.parse(changed, other, "instrumented")

    def test_session_close_refusal_and_mapping_owner_actions_are_public_oracles(self):
        for workload, key, field in (("lifecycle-session-bgra-roi", "lifecycle", "closed_search_refused"),
                                     ("lifecycle-session-rgba-roi", "lifecycle", "retained_after_close"),
                                     ("session-owner8-bgra-roi", "setup", "mapping_created"),
                                     ("session-owner8-rgba-roi", "teardown", "mapping_released")):
            value = v3_payload(workload, smoke=True)
            value["workloads"][0]["unit_observations"][0][key][field] = False
            with self.subTest(workload=workload, field=field), self.assertRaises(ValueError):
                self.parse(value, workload)

    def test_role_work_counts_are_not_an_arbitrary_correct_flag(self):
        workload = "session-group8-rgba-distinct-roi"
        for key in ("backend_find_calls", "backend_find_completions", "bgr_conversions", "bgr_converted_bytes",
                    "mapping_copies", "mapping_copied_bytes"):
            value = v3_payload(workload, "instrumented", smoke=True)
            value["workloads"][0]["unit_observations"][0]["work"][key] -= 1
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.parse(value, workload, "instrumented")

    def test_retained_report_revalidates_units_and_apparatus_without_live_source_reads(self):
        workload = "query-match-rgba-roi"
        directory = self.root / "retained"
        persist_v3_smoke(directory, workload)
        original = collector.report(directory)
        self.assertEqual(original["status"], "passed")
        self.assertFalse(original["performance_qualified"])
        path = directory / "attempts/0001.json"
        raw = path.read_bytes()
        for change in ("query", "apparatus"):
            attempt = collector.strict_json(raw.decode())
            value = collector.strict_json(attempt["record"]["stdout"])
            if change == "query":
                value["workloads"][0]["unit_observations"][0]["query"]["after_close_same"] = False
            else:
                value["apparatus"]["source_files"][collector.APPARATUS_SOURCES[-1]] = "9" * 64
            attempt["record"]["stdout"] = json.dumps(value)
            path.write_bytes(collector.document(attempt))
            report = collector.report(directory)
            self.assertEqual(report["status"], "failed")
            self.assertIn("semantic", {issue.get("category") for issue in report["issues"]})
        path.write_bytes(raw)
        self.assertEqual(collector.report(directory)["status"], "passed")

    def test_current_smoke_uses_real_collection_and_preserves_independent_failure_classes(self):
        workloads = ["session-retained-rgba-roi-hit", "query-deadline-rgba-roi"]
        args = collector.parser().parse_args([
            "smoke", "--protocol-version", "3", "--build-mode", "normal", "--target", TARGET,
            "--baseline", "baseline", "--baseline-metadata", "build.json",
            "--output", str(self.root / "smoke")])
        records = [process_record(v3_manifest())]
        for workload in collector.definitions_for("normal", 3):
            value = v3_payload(workload, smoke=True)
            record = process_record(value)
            if workload == workloads[0]:
                value["workloads"][0]["unit_observations"][0]["projections"] = []
                record = process_record(value)
                record["exit_code"] = 23
            elif workload == "session-retained-bgra-roi-hit":
                value["workloads"][0]["unit_observations"][0]["projections"][0]["matches"][0]["score"] = 10**400
                record = process_record(value)
            records.append(record)
        original_runtime = collector.loaded_runtime
        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "binary_identity", return_value=v3_identity(self.root)),
              mock.patch.object(collector, "check_files"),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector, "loaded_runtime",
                                side_effect=lambda value, identity=None, **kwargs: original_runtime(value, identity)),
              mock.patch.object(collector, "run_process", side_effect=records) as launch):
            result = collector.collect(args, "smoke")
        self.assertEqual(launch.call_count, 55)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(launch.call_args_list[0].args[0][1:], ["--protocol-version", "3", "--list"])
        self.assertTrue({"exit", "semantic"} <= {issue.get("category") for issue in result["issues"]})
        self.assertEqual(sum(issue.get("category") == "semantic" for issue in result["issues"]), 2)
        self.assertFalse(any(issue["status"] == "not-run" for issue in result["issues"]))
        self.assertIn(workloads[1], result["process_observations"]["baseline"])


class ProtocolThreeGradingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.root = Path(cls.temporary.name).resolve()
        cls.calibration = v3_parsed_evidence(cls.root, "calibration")
        cls.control = v3_parsed_evidence(cls.root, "control")
        cls.comparison = v3_parsed_evidence(cls.root, "comparison")
        cls.profile = v2_numeric_profile(cls.calibration)
        cls.profile["format_version"] = 3

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def test_all_declared_strata_enter_calibration_controls_and_process_ceilings(self):
        workload = "session-turnover-rgba-roi"
        envelope = self.profile["envelopes"][workload]
        for name in ("hit", "miss"):
            for metric in ("p95_ns", "cpu_ns"):
                self.assertIn(f"strata.{name}.{metric}", envelope["limits"])
                self.assertIn(f"strata.{name}.{metric}", envelope["benefit_allowances"])
        self.assertEqual(collector.control_evidence(self.calibration, self.control), [])
        issues, benefits = collector.compare_evidence(self.comparison, self.profile)
        self.assertEqual(issues, [])
        self.assertNotIn(workload, benefits)
        for role in ("baseline", "candidate"):
            changed = copy.deepcopy(self.comparison)
            metric = "strata.hit.p95_ns"
            changed["observations"][role][workload][-1][metric] = envelope["limits"][metric] + 1
            issues, _ = collector.compare_evidence(changed, self.profile)
            self.assertTrue(any(issue["attempt"] == f"{role}:{workload}:5" and metric in issue["reason"]
                                for issue in issues))

    def test_missing_stratum_is_never_ignored_at_any_future_gated_stage(self):
        workload = "session-turnover-bgra-full"
        for metric in ("strata.hit.p95_ns", "strata.miss.cpu_ns"):
            calibration = copy.deepcopy(self.calibration)
            del calibration["observations"]["baseline"][workload][-1][metric]
            with self.subTest(metric=metric), self.assertRaises(ValueError):
                collector.derive_envelopes(calibration)
            control = copy.deepcopy(self.control)
            del control["observations"]["control"][workload][-1][metric]
            self.assertTrue(any(issue["status"] == "incomplete" for issue in
                                collector.control_evidence(self.calibration, control)))
            comparison = copy.deepcopy(self.comparison)
            del comparison["observations"]["candidate"][workload][-1][metric]
            self.assertTrue(any(issue["status"] == "incomplete" for issue in
                                collector.compare_evidence(comparison, self.profile)[0]))

    def test_each_stratum_has_independent_control_noise_order_drift_and_ceiling_checks(self):
        workload = "session-turnover-rgba-full"
        for metric in ("strata.hit.p95_ns", "strata.miss.cpu_ns"):
            for kind in ("noise", "order", "drift", "ceiling"):
                control = copy.deepcopy(self.control)
                rows = control["observations"]["control"][workload]
                if kind == "noise":
                    rows[-1][metric] *= 4
                elif kind == "ceiling":
                    rows[-1][metric] = self.profile["envelopes"][workload]["limits"][metric] + 1
                else:
                    allowance = self.profile["envelopes"][workload]["benefit_allowances"][metric]
                    for row in rows:
                        if (row["position"] == 0 if kind == "order" else row["process"] >= 3):
                            row[metric] += allowance + 1
                issues = collector.control_evidence(self.calibration, control)
                with self.subTest(metric=metric, kind=kind):
                    self.assertTrue(any(metric in issue["reason"] and
                                        (kind in issue["reason"] or kind == "ceiling" and "exceeds" in issue["reason"])
                                        for issue in issues))

    def test_stratum_control_bias_cannot_pass_as_unchanged(self):
        workload = "session-turnover-rgba-full"
        for metric in ("strata.hit.p95_ns", "strata.miss.cpu_ns"):
            calibration = copy.deepcopy(self.calibration)
            for row in calibration["observations"]["baseline"][workload]:
                row[metric] = 100_000
            allowance = collector.derive_envelopes(calibration)[workload]["benefit_allowances"][metric]
            for role in ("baseline", "control"):
                with self.subTest(metric=metric, role=role):
                    control = copy.deepcopy(self.control)
                    for label in ("baseline", "control"):
                        for row in control["observations"][label][workload]:
                            row[metric] = 100_000 - (allowance + 1 if label == role else 0)
                    control["observations"][role][workload][0]["peak_rss_bytes"] = None
                    issues = collector.control_evidence(calibration, control)
                    self.assertTrue(any(issue["status"] == "failed" for issue in issues))
                    self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))

    def test_each_comparison_stratum_keeps_its_own_noise_cap(self):
        workload = "session-turnover-rgba-full"
        for metric in ("strata.hit.p95_ns", "strata.miss.cpu_ns"):
            calibration = copy.deepcopy(self.calibration)
            for row in calibration["observations"]["baseline"][workload]:
                row[metric] = 100_000
            profile = v2_numeric_profile(calibration)
            profile["format_version"] = 3
            for role in ("baseline", "candidate"):
                for excess in (0, 1):
                    with self.subTest(metric=metric, role=role, excess=excess):
                        comparison = copy.deepcopy(self.comparison)
                        other = "candidate" if role == "baseline" else "baseline"
                        for row in comparison["observations"][other][workload]:
                            row[metric] = 100_000
                        for index, row in enumerate(comparison["observations"][role][workload]):
                            row[metric] = 80_000 if index < 4 else 92_000 + excess
                        issues, _ = collector.compare_evidence(comparison, profile)
                        self.assertEqual(bool(issues), bool(excess))
                        self.assertTrue(all(issue["status"] == "failed" for issue in issues))

    def test_observed_role_clock_cannot_relax_frozen_cohort_noise(self):
        cases = (("single-small96x64", "p95_ns"), ("single-small96x64", "cpu_ns"),
                 ("session-turnover-rgba-full", "strata.hit.p95_ns"),
                 ("session-turnover-rgba-full", "strata.miss.cpu_ns"))
        for workload, metric in cases:
            clock = "timer_resolution_ns" if metric.endswith("p95_ns") else "cpu_timer_resolution_ns"
            calibration = copy.deepcopy(self.calibration)
            for row in calibration["observations"]["baseline"][workload]:
                row[metric], row[clock] = 100_000, 1_000
            profile = v2_numeric_profile(calibration)
            profile["format_version"] = 3
            for mode, original in (("control", self.control), ("comparison", self.comparison)):
                for role in original["observations"]:
                    with self.subTest(workload=workload, metric=metric, mode=mode, role=role):
                        evidence = copy.deepcopy(original)
                        for rows in evidence["observations"].values():
                            for row in rows[workload]:
                                row[metric], row[clock] = 100_000, 1_000
                        rows = evidence["observations"][role][workload]
                        rows[0][metric] = 84_999
                        for row in rows:
                            row[clock] = 200_000
                        issues = (collector.control_evidence(calibration, evidence) if mode == "control"
                                  else collector.compare_evidence(evidence, profile)[0])
                        self.assertEqual(len(issues), 1)
                        self.assertEqual(issues[0]["status"], "failed")

    def test_unavailable_clock_keeps_known_between_role_cpu_bias(self):
        for workload, metric in (("single-small96x64", "cpu_ns"),
                                 ("session-turnover-rgba-full", "strata.miss.cpu_ns")):
            calibration = copy.deepcopy(self.calibration)
            for row in calibration["observations"]["baseline"][workload]:
                row[metric] = 100_000
            allowance = collector.derive_envelopes(calibration)[workload]["benefit_allowances"][metric]
            for role in ("baseline", "control"):
                with self.subTest(workload=workload, role=role):
                    control = copy.deepcopy(self.control)
                    for label in control["observations"]:
                        for row in control["observations"][label][workload]:
                            row[metric] = 100_000 - (allowance + 1 if label == role else 0)
                    row = control["observations"][role][workload][0]
                    row["cpu_timer_resolution_ns"] = None
                    row["unavailable"] = ["cpu_timer_resolution_ns"]
                    issues = collector.control_evidence(calibration, control)
                    self.assertTrue(any(issue["status"] == "failed" for issue in issues))
                    self.assertTrue(any(issue["status"] == "incomplete" for issue in issues))

    def test_independent_cache_allowance_changes_only_its_rss_peak(self):
        independent = self.profile["envelopes"]["lifecycle-independent-rgba-4k"]["limits"]
        session = self.profile["envelopes"]["lifecycle-session-rgba-roi"]["limits"]
        self.assertEqual(independent["peak_rss_bytes"] - session["peak_rss_bytes"], 32 * collector.MIB)
        for metric in ("rss_growth_bytes", "rss_residual_bytes"):
            self.assertEqual(independent[metric], session[metric])
            self.assertEqual(independent[metric], collector.MIB)

    def test_unchanged_baseline_and_control_apparatus_remain_mandatory(self):
        for field in ("baseline", "control"):
            control = copy.deepcopy(self.control)
            control["plan"]["binaries"][field]["build"]["apparatus"]["source_files"][collector.APPARATUS_SOURCES[0]] = "9" * 64
            with self.subTest(field=field), self.assertRaises(ValueError):
                collector.control_evidence(self.calibration, control)

    def test_full_corpus_report_retains_normal_performance_verdict(self):
        directory = self.root / "full-report"
        directory.mkdir()
        collector.save(directory / "profile.json", self.profile)
        evidence = copy.deepcopy(self.comparison)
        evidence["plan"]["profile_sha256"] = collector.sha256(directory / "profile.json")
        with (mock.patch.object(collector, "evaluate", return_value=evidence),
              mock.patch.object(collector, "validate_profile", return_value=self.profile)):
            result = collector.report(directory)
        self.assertEqual(result["status"], "passed")
        self.assertTrue(result["performance_qualified"])
        self.assertEqual(result["targets"][TARGET], "passed")
        self.assertEqual(result["normal_timing_verdict"], "passed")
        self.assertEqual(result["heap_and_work_verdict"], "unavailable")
        self.assertEqual(result["whole_change_acceptance"], "incomplete")
        self.assertEqual(result["workload_coverage"],
                         {"required": self.comparison["plan"]["workloads"],
                          "selected": self.comparison["plan"]["workloads"], "omitted": [], "complete": True})

    def test_missing_method_review_blocks_current_calibration_before_output_or_launch(self):
        args = collector.parser().parse_args([
            "calibrate", "--protocol-version", "3", "--build-mode", "normal", "--target", TARGET,
            "--baseline", "baseline", "--baseline-metadata", "build.json", "--exclusive-host",
            "--output", str(self.root / "denied")])
        with (mock.patch.object(collector, "host_identity", return_value=self.calibration["plan"]["host"]),
              mock.patch.object(collector, "binary_identity", return_value=v3_identity(self.root)),
              mock.patch.object(collector, "run_process") as launch, self.assertRaises(ValueError)):
            collector.collect(args, "calibration")
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())


class WorkloadCohortTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.workload = "selection-k16-dense1080p-keepall"
        self.caps = {"calibration": 1200, "control": 1800, "comparison": 2400}

    def cohorts(self, build_mode="normal", workload=None):
        workload = workload or self.workload
        root = self.root / f"{build_mode}-{workload}"
        root.mkdir()
        cohorts = tuple(v3_parsed_evidence(root, mode, build_mode, workload)
                        for mode in ("calibration", "control", "comparison"))
        for evidence in cohorts:
            plan = evidence["plan"]
            plan["runtime_observer_policy"] = "macos-dyld-observer-on-warm-v1"
            plan["binaries"] = {role: timing_identity(binary) for role, binary in plan["binaries"].items()}
            evidence["manifest"] = plan["binaries"]["baseline"]["build"]["benchmark_manifest"]
        binding = save_method_review(root, cohorts[2]["plan"], self.caps)
        for evidence in cohorts:
            evidence["plan"]["method_review"] = copy.deepcopy(binding)
            evidence["plan"]["execution_limits"] = copy.deepcopy(binding["stage_limits"][evidence["plan"]["mode"]])
        return cohorts

    def freeze(self, calibration, control):
        plan = calibration["plan"]
        root = Path(plan["method_review"]["path"]).parent
        review = {"format_version": 3, "kind": "component-profile-review", "decision": "accepted",
                  "target": TARGET, "build_mode": plan["build_mode"], "workloads": plan["workloads"],
                  "runtime_observer_policy": plan["runtime_observer_policy"],
                  "method_review": plan["method_review"], "calibration": collector.cohort_binding(calibration),
                  "control": collector.cohort_binding(control), "envelopes": collector.derive_envelopes(calibration),
                  "reviewer": "fixture reviewer", "rationale": "synthetic scoped arithmetic only"}
        path = root / "profile-review.json"
        collector.save(path, review)
        with mock.patch.object(collector, "runtime_closure_admission"):
            return collector.profile_from_v2(calibration, root / "calibration", control, root / "control",
                                             "fixture reviewer", "synthetic scoped arithmetic only",
                                             path, collector.sha256(path))

    def arguments(self, evidence, profile=None, suffix="run"):
        plan = evidence["plan"]
        root = Path(plan["method_review"]["path"]).parent
        name = {"calibration": "calibrate", "control": "control", "comparison": "compare"}[plan["mode"]]
        argv = [name, "--protocol-version", "3", "--build-mode", plan["build_mode"], "--target", TARGET, "--baseline", "baseline",
                "--baseline-metadata", "baseline.json", "--exclusive-host",
                "--method-review", plan["method_review"]["path"],
                "--method-review-sha256", plan["method_review"]["sha256"],
                "--output", str(root / f"{plan['mode']}-{suffix}")]
        if len(plan["workloads"]) == 1:
            argv += ["--workload", plan["workloads"][0]]
        if name == "control":
            argv += ["--calibration", str(root / "calibration"), "--calibration-plan-sha256", "1" * 64]
        if name == "compare":
            path = root / f"profile-{suffix}.json"
            collector.save(path, profile)
            argv += ["--candidate", "candidate", "--candidate-metadata", "candidate.json",
                     "--profile", str(path), "--profile-sha256", collector.sha256(path)]
        return collector.parser().parse_args(argv)

    @contextlib.contextmanager
    def native_context(self, cohorts, records=(), clock=None):
        # Native provenance/timing already have independent tests; these doubles
        # isolate scheduling, immutable bindings and grading without native launches.
        calibration, control, comparison = cohorts
        plan = comparison["plan"]
        root = Path(plan["method_review"]["path"]).parent
        retained = {root / "calibration": calibration, root / "control": control}
        original_evaluate, original_runtime = collector.evaluate, collector.loaded_runtime
        runtime = {"plan": {"host": plan["host"], "environment": plan["environment"]},
                   "threads": collector.effective_threads(v3_manifest(plan["build_mode"])),
                   "closure": {}, "raw": b"{}"}
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(collector, "host_identity", return_value=plan["host"]))
            stack.enter_context(mock.patch.object(collector, "environment", return_value=plan["environment"]))
            stack.enter_context(mock.patch.object(
                collector, "binary_identity", side_effect=lambda role, *args: plan["binaries"][role]))
            stack.enter_context(mock.patch.object(
                collector, "evaluate", side_effect=lambda path: retained[Path(path)]
                if Path(path) in retained else original_evaluate(path)))
            stack.enter_context(mock.patch.object(collector, "runtime_closure_admission", return_value=runtime))
            stack.enter_context(mock.patch.object(collector, "validate_runtime_attempt"))
            stack.enter_context(mock.patch.object(collector, "validate_timing_attempt"))
            stack.enter_context(mock.patch.object(collector, "check_files"))
            stack.enter_context(mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(
                collector, "loaded_runtime",
                side_effect=lambda value, identity=None, **kwargs: original_runtime(value, identity)))
            if clock is not None:
                stack.enter_context(mock.patch.object(collector.time, "monotonic", side_effect=clock))
            observe = stack.enter_context(mock.patch.object(
                collector.macos_images, "observe", side_effect=({"record": record} for record in records)))
            launch = stack.enter_context(mock.patch.object(collector, "run_process"))
            yield observe, launch

    def records(self, evidence):
        plan = evidence["plan"]
        for task in plan["schedule"]:
            role = "candidate" if task["role"] == "candidate" else "baseline"
            value = (v3_manifest(plan["build_mode"], role) if task["workload"] is None else
                     v3_payload(task["workload"], plan["build_mode"], role=role, candidate=role == "candidate"))
            value.update(timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=True)
            yield process_record(value)

    @contextlib.contextmanager
    def replace_profile_before_read(self, path, replacement):
        original_bytes, original_text = Path.read_bytes, Path.read_text

        def read_bytes(current):
            if current == path:
                current.write_bytes(replacement)
            return original_bytes(current)

        def read_text(current, *args, **kwargs):
            if current == path:
                current.write_bytes(replacement)
            return original_text(current, *args, **kwargs)

        with (mock.patch.object(Path, "read_bytes", read_bytes),
              mock.patch.object(Path, "read_text", read_text)):
            yield

    def test_comparison_profile_read_cannot_replace_reviewed_bytes(self):
        cohorts = self.cohorts()
        profile = self.freeze(*cohorts[:2])
        args = self.arguments(cohorts[2], profile)
        replacement = collector.document({**profile, "rationale": "unreviewed replacement"})
        with (self.native_context(cohorts, self.records(cohorts[2])) as (observe, launch),
              self.replace_profile_before_read(Path(args.profile), replacement),
              self.assertRaisesRegex(ValueError, "reviewed profile hash mismatch")):
            collector.collect(args, "comparison")
        observe.assert_not_called()
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_profile_review_read_cannot_replace_rejected_decision(self):
        cohorts = self.cohorts()
        profile = self.freeze(*cohorts[:2])
        path = Path(profile["profile_review"]["path"])
        accepted = path.read_bytes()
        review = collector.load(path)
        path.write_bytes(collector.document({**review, "decision": "rejected"}))
        rejected_hash = collector.sha256(path)
        with (self.native_context(cohorts),
              self.replace_profile_before_read(path, accepted),
              self.assertRaisesRegex(ValueError, "profile review hash mismatch")):
            collector.profile_from_v2(cohorts[0], path.parent / "calibration",
                                      cohorts[1], path.parent / "control",
                                      "fixture reviewer", "synthetic scoped arithmetic only",
                                      path, rejected_hash)

    def test_report_profile_read_cannot_replace_frozen_bytes(self):
        cohorts = self.cohorts()
        profile = self.freeze(*cohorts[:2])
        directory = self.root / "retained-report"
        directory.mkdir()
        path = directory / "profile.json"
        collector.save(path, profile)
        comparison = cohorts[2]
        comparison["plan"]["profile_sha256"] = collector.sha256(path)
        replacement = collector.document({**profile, "rationale": "unreviewed replacement"})
        with self.native_context(cohorts):
            original_evaluate = collector.evaluate
            with (mock.patch.object(collector, "evaluate", side_effect=lambda current:
                                    comparison if Path(current) == directory else original_evaluate(current)),
                  self.replace_profile_before_read(path, replacement)):
                result = collector.report(directory)
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["performance_qualified"])
        self.assertTrue(any(issue["reason"] == "frozen profile hash mismatch"
                            for issue in result["issues"]))

    def test_single_workload_collection_retains_all_processes_counts_and_reviewed_limits(self):
        for workload in (self.workload, "lifecycle-session-rgba-roi"):
            cohorts = self.cohorts(workload=workload)
            profile = self.freeze(*cohorts[:2])
            for evidence in cohorts:
                mode = evidence["plan"]["mode"]
                args = self.arguments(evidence, profile if mode == "comparison" else None)
                with self.subTest(workload=workload, mode=mode), self.native_context(
                        cohorts, self.records(evidence)) as (observe, launch):
                    result = collector.collect(args, mode)
                    retained = collector.load(Path(args.output) / "plan.json")
                    collector.validate_plan(retained)
                launch.assert_not_called()
                self.assertEqual(observe.call_count, 7 if mode == "calibration" else 14)
                self.assertEqual(retained["workloads"], [workload])
                self.assertEqual(retained["execution_limits"], retained["method_review"]["stage_limits"][mode])
                self.assertEqual(retained["execution_limits"]["campaign_timeout_seconds"], self.caps[mode])
                self.assertEqual(len(result["attempts"]), observe.call_count)
                self.assertEqual(result["status"], "passed")
                self.assertFalse(result["performance_qualified"])
                for rows in result["process_observations"].values():
                    self.assertEqual(list(rows), [workload])
                    self.assertEqual([row["process"] for row in rows[workload]], list(range(6)))
                tasks = retained["schedule"]
                roles = ("baseline",) if mode == "calibration" else (
                    "baseline", "control" if mode == "control" else "candidate")
                self.assertEqual([task["role"] for task in tasks[:len(roles)]], list(roles))
                self.assertEqual([task["role"] for task in tasks[len(roles):]],
                                 [role for index in range(6) for role in
                                  (roles[::-1] if index % 2 else roles)])
                self.assertEqual([task["position"] for task in tasks[len(roles):]],
                                 [0] * 6 if mode == "calibration" else [0, 1] * 6)
                for task in tasks[len(roles):]:
                    attempt = collector.load(Path(args.output) / "attempts" / f"{task['id']}.json")
                    value = collector.strict_json(attempt["record"]["stdout"])["workloads"][0]
                    self.assertEqual((value["warmups"], value["sample_count"]),
                                     (20, 200) if workload == self.workload else (1, 3))
                    self.assertNotIn("--smoke", attempt["argv"])
                self.assertEqual(profile["workloads"], [workload])
                self.assertEqual(list(profile["envelopes"]), [workload])

    def test_selected_campaign_pause_consumes_original_reviewed_deadline(self):
        cohorts = self.cohorts()
        args = self.arguments(cohorts[0])
        with self.native_context(cohorts, self.records(cohorts[0]),
                                 clock=[0, 0, 0, 1, 1201]) as (observe, launch):
            with contextlib.closing(collector.collection_steps(args, "calibration")) as steps:
                checkpoint = next(steps)
                raw = Path(checkpoint["path"]).read_bytes()
                self.assertEqual(checkpoint["campaign_deadline_monotonic"], 1200)
                args.workload = "single-small96x64"
                with self.assertRaises(StopIteration) as stopped:
                    next(steps)
            result = stopped.exception.value
        self.assertEqual(observe.call_count, 1)
        launch.assert_not_called()
        self.assertEqual(result["status"], "incomplete")
        self.assertEqual(result["workload_coverage"]["selected"], [self.workload])
        self.assertEqual(len(result["issues"]), 6)
        self.assertTrue(all(issue["status"] == "not-run" for issue in result["issues"]))
        self.assertEqual(Path(checkpoint["path"]).read_bytes(), raw)
        self.assertFalse((Path(args.output) / "started/0001.json").exists())

    def test_collected_plan_cap_cannot_be_enlarged_before_control(self):
        cohorts = self.cohorts()
        args = self.arguments(cohorts[0])
        with self.native_context(cohorts, self.records(cohorts[0])):
            collector.collect(args, "calibration")
        path = Path(args.output) / "plan.json"
        plan = collector.load(path)
        plan["execution_limits"]["campaign_timeout_seconds"] += 1
        path.write_bytes(collector.document(plan))
        control_args = self.arguments(cohorts[1])
        control_args.calibration = args.output
        control_args.calibration_plan_sha256 = collector.sha256(path)
        with self.native_context(cohorts) as (observe, launch):
            with self.assertRaisesRegex(ValueError, "bounded v2 execution policy"):
                collector.collect(control_args, "control")
        observe.assert_not_called()
        launch.assert_not_called()
        self.assertFalse(Path(control_args.output).exists())

    def test_calibration_requires_exact_reviewed_scope_before_launch(self):
        cohorts = self.cohorts()
        for workload in (None, "single-small96x64"):
            args = self.arguments(cohorts[0])
            args.workload = workload
            with self.subTest(workload=workload), self.native_context(cohorts) as (observe, launch):
                with self.assertRaisesRegex(ValueError, "method review workloads mismatch"):
                    collector.collect(args, "calibration")
            observe.assert_not_called()
            launch.assert_not_called()
            self.assertFalse(Path(args.output).exists())

    def test_repeated_non_smoke_workload_options_never_select_a_last_winner(self):
        for command in ("calibrate", "control", "compare", "smoke"):
            argv = [command, "--build-mode", "normal", "--target", TARGET,
                    "--baseline", "baseline", "--baseline-metadata", "build.json",
                    "--output", str(self.root / "denied"), "--exclusive-host"]
            if command == "control":
                argv += ["--calibration", "calibration", "--calibration-plan-sha256", "1" * 64]
            if command == "compare":
                argv += ["--candidate", "candidate", "--candidate-metadata", "candidate.json",
                         "--profile", "profile.json", "--profile-sha256", "2" * 64]
            for second in (self.workload, "single-small96x64"):
                repeated = argv + ["--workload", self.workload, "--workload", second]
                if command == "smoke":
                    self.assertEqual(collector.parser().parse_args(repeated).workload, second)
                else:
                    with (self.subTest(command=command, second=second),
                          mock.patch.object(sys, "stderr"),
                          mock.patch.object(collector.macos_images, "observe") as observe,
                          mock.patch.object(collector, "run_process") as launch,
                          self.assertRaises(SystemExit)):
                        collector.main(repeated)
                    observe.assert_not_called()
                    launch.assert_not_called()
                    self.assertFalse((self.root / "denied").exists())

    def test_only_reviewed_integer_campaign_caps_can_change_in_each_stage(self):
        cohorts = self.cohorts()
        specification = cohorts[0]["plan"]
        path = Path(specification["method_review"]["path"])
        original = path.read_bytes()
        for mode in self.caps:
            for value in (0, -1, True, 1.5, "1200", None, float("inf"), float("nan"),
                          (21600 if mode == "calibration" else 43200) + 1):
                review = collector.strict_json(original.decode())
                review["stage_limits"][mode]["campaign_timeout_seconds"] = value
                path.write_text(json.dumps(review), encoding="utf-8")
                args = self.arguments(cohorts[0])
                args.method_review_sha256 = collector.sha256(path)
                with self.subTest(mode=mode, value=value), self.native_context(cohorts) as (observe, launch):
                    with self.assertRaises(ValueError):
                        collector.collect(args, "calibration")
                observe.assert_not_called()
                launch.assert_not_called()
                self.assertFalse(Path(args.output).exists())
            for field in ("child_timeout_seconds", "cleanup_seconds", "output_limit_bytes",
                          "runtime_binding_seconds", "max_attempts", "campaign_output_limit_bytes"):
                review = collector.strict_json(original.decode())
                review["stage_limits"][mode][field] += 1
                path.write_bytes(collector.document(review))
                args = self.arguments(cohorts[0])
                args.method_review_sha256 = collector.sha256(path)
                with self.subTest(mode=mode, field=field), self.native_context(cohorts) as (observe, launch):
                    with self.assertRaisesRegex(ValueError, "finite cost/bounds"):
                        collector.collect(args, "calibration")
                observe.assert_not_called()
                launch.assert_not_called()
                self.assertFalse(Path(args.output).exists())
        for cap in (1, 21600):
            review = collector.strict_json(original.decode())
            review["stage_limits"]["calibration"]["campaign_timeout_seconds"] = cap
            path.write_bytes(collector.document(review))
            binding = collector.reviewed_method(path, collector.sha256(path), specification)
            self.assertEqual(binding["stage_limits"]["calibration"]["campaign_timeout_seconds"], cap)

    def test_method_limits_are_taken_from_the_hashed_review_bytes(self):
        calibration, _, _ = self.cohorts()
        specification = calibration["plan"]
        path = Path(specification["method_review"]["path"])
        original = path.read_bytes()
        changed = collector.strict_json(original.decode())
        changed["stage_limits"]["calibration"]["campaign_timeout_seconds"] += 1
        original_read = Path.read_bytes

        def replace_after_read(current):
            raw = original_read(current)
            if current == path:
                path.write_bytes(collector.document(changed))
            return raw

        with mock.patch.object(Path, "read_bytes", replace_after_read):
            binding = collector.reviewed_method(path, specification["method_review"]["sha256"], specification)
        self.assertEqual(binding, specification["method_review"])
        with self.assertRaisesRegex(ValueError, "method review hash mismatch"):
            collector.reviewed_method(path, binding["sha256"], specification)

    def test_retained_scope_schedule_and_limits_cannot_be_rebound(self):
        cohorts = self.cohorts()
        for evidence in cohorts:
            collector.validate_plan(evidence["plan"])
            for change in ("missing-scope", "other-scope", "expanded", "duplicate", "all", "order",
                           "plan-limit", "binding-limit", "missing-binding-limits"):
                changed = copy.deepcopy(evidence["plan"])
                if change == "missing-scope":
                    del changed["workloads"]
                elif change in ("other-scope", "expanded", "duplicate", "all"):
                    changed["workloads"] = {
                        "other-scope": ["single-small96x64"], "expanded": [self.workload, "single-small96x64"],
                        "duplicate": [self.workload, self.workload],
                        "all": list(collector.definitions_for("normal", 3))}[change]
                    changed["schedule"] = collector.schedule(changed["mode"], changed["workloads"], 3)
                elif change == "order":
                    changed["schedule"].reverse()
                elif change == "plan-limit":
                    changed["execution_limits"]["campaign_timeout_seconds"] += 1
                elif change == "binding-limit":
                    changed["method_review"]["stage_limits"]["comparison"]["campaign_timeout_seconds"] += 1
                else:
                    del changed["method_review"]["stage_limits"]
                with self.subTest(mode=evidence["plan"]["mode"], change=change), self.assertRaises(ValueError):
                    collector.validate_plan(changed)

    def test_older_protocol_and_wrong_mode_selections_refuse_before_native_launch(self):
        for version, workload in ((1, self.workload), (2, self.workload), (3, "watch-exact-rgba")):
            for command, mode in (("calibrate", "calibration"), ("control", "control"), ("compare", "comparison")):
                argv = [command, "--protocol-version", str(version), "--build-mode", "normal",
                        "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "build.json",
                        "--workload", workload, "--output", str(self.root / "denied"), "--exclusive-host"]
                if mode == "control":
                    argv += ["--calibration", "calibration", "--calibration-plan-sha256", "1" * 64]
                if mode == "comparison":
                    argv += ["--candidate", "candidate", "--candidate-metadata", "candidate.json",
                             "--profile", "profile.json", "--profile-sha256", "2" * 64]
                args = collector.parser().parse_args(argv)
                with (self.subTest(version=version, mode=mode),
                      mock.patch.object(collector.macos_images, "observe") as observe,
                      mock.patch.object(collector, "run_process") as launch,
                      self.assertRaises(ValueError)):
                    collector.collect(args, mode)
                observe.assert_not_called()
                launch.assert_not_called()
                self.assertFalse(Path(args.output).exists())

    def test_control_refuses_independently_reviewed_cross_scope_or_changed_caps(self):
        cohorts = self.cohorts()
        original = collector.load(cohorts[0]["plan"]["method_review"]["path"])
        for change in ("other", "all", "cap"):
            review = copy.deepcopy(original)
            if change != "cap":
                review["workloads"] = (["single-small96x64"] if change == "other"
                                       else list(collector.definitions_for("normal", 3)))
                review["stage_limits"] = {mode: collector.execution_limits(
                    mode, len(collector.schedule(mode, review["workloads"], 3))) for mode in self.caps}
            else:
                review["stage_limits"]["control"]["campaign_timeout_seconds"] += 1
            path = self.root / f"{change}-method.json"
            collector.save(path, review)
            args = self.arguments(cohorts[1])
            args.workload = review["workloads"][0] if change != "all" else None
            args.method_review, args.method_review_sha256 = str(path), collector.sha256(path)
            with self.subTest(change=change), self.native_context(cohorts) as (observe, launch):
                with self.assertRaisesRegex(ValueError, "control workloads|control method_review"):
                    collector.collect(args, "control")
            observe.assert_not_called()
            launch.assert_not_called()
            self.assertFalse(Path(args.output).exists())

    def test_profile_scope_envelopes_and_stage_limits_are_frozen_before_comparison_launch(self):
        cohorts = self.cohorts()
        profile = self.freeze(*cohorts[:2])
        for change in ("missing", "other", "expanded", "missing-envelope", "extra-envelope", "cap", "numeric"):
            changed = copy.deepcopy(profile)
            if change == "missing":
                del changed["workloads"]
            elif change in ("other", "expanded"):
                changed["workloads"] = ["single-small96x64"] if change == "other" else [
                    self.workload, "single-small96x64"]
            elif change == "missing-envelope":
                changed["envelopes"].clear()
            elif change == "extra-envelope":
                changed["envelopes"]["single-small96x64"] = copy.deepcopy(changed["envelopes"][self.workload])
            elif change == "cap":
                changed["method_review"]["stage_limits"]["comparison"]["campaign_timeout_seconds"] += 1
            else:
                changed["envelopes"][self.workload]["limits"]["rss_growth_bytes"] += 1
            args = self.arguments(cohorts[2], changed, suffix=change)
            with self.subTest(change=change), self.native_context(cohorts) as (observe, launch):
                with self.assertRaisesRegex(ValueError, "profile/provenance/envelopes"):
                    collector.collect(args, "comparison")
            observe.assert_not_called()
            launch.assert_not_called()
            self.assertFalse(Path(args.output).exists())
        for workload in (None, "single-small96x64"):
            args = self.arguments(cohorts[2], profile, suffix=str(workload))
            args.workload = workload
            with self.subTest(workload=workload), self.native_context(cohorts) as (observe, launch):
                with self.assertRaisesRegex(ValueError, "comparison workloads"):
                    collector.collect(args, "comparison")
            observe.assert_not_called()
            launch.assert_not_called()
            self.assertFalse(Path(args.output).exists())
        review = collector.load(profile["method_review"]["path"])
        review["stage_limits"]["comparison"]["campaign_timeout_seconds"] += 1
        path = self.root / "changed-cap.json"
        collector.save(path, review)
        args = self.arguments(cohorts[2], profile, suffix="changed-cap")
        args.method_review, args.method_review_sha256 = str(path), collector.sha256(path)
        with self.native_context(cohorts) as (observe, launch):
            with self.assertRaisesRegex(ValueError, "comparison build mode/method review changed"):
                collector.collect(args, "comparison")
        observe.assert_not_called()
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_external_profile_review_must_bind_explicit_scope_and_all_stage_caps(self):
        cohorts = self.cohorts()
        profile = self.freeze(*cohorts[:2])
        path = Path(profile["profile_review"]["path"])
        original = path.read_bytes()
        for change in ("missing", "other", "extra-envelope", "cap"):
            review = collector.strict_json(original.decode())
            if change == "missing":
                del review["workloads"]
            elif change == "other":
                review["workloads"] = ["single-small96x64"]
            elif change == "extra-envelope":
                review["envelopes"]["single-small96x64"] = copy.deepcopy(review["envelopes"][self.workload])
            else:
                review["method_review"]["stage_limits"]["comparison"]["campaign_timeout_seconds"] += 1
            path.write_bytes(collector.document(review))
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, "exact method/cohorts/envelopes"):
                collector.profile_from_v2(cohorts[0], profile["calibration"]["path"], cohorts[1],
                                          profile["control"]["path"], "reviewer", "scoped fixture",
                                          path, collector.sha256(path))

    def test_partial_report_preserves_scoped_pass_incompleteness_and_known_failures(self):
        for build_mode in ("normal", "instrumented"):
            cohorts = self.cohorts(build_mode)
            calibration, control, comparison = cohorts
            profile = self.freeze(calibration, control)
            directory = Path(profile["method_review"]["path"]).parent / "report"
            directory.mkdir()
            collector.save(directory / "profile.json", profile)
            for condition in ("passed", "incomplete", "failed"):
                evidence = copy.deepcopy(comparison)
                evidence["plan"]["profile_sha256"] = collector.sha256(directory / "profile.json")
                if condition != "passed":
                    evidence["observations"]["baseline"][self.workload].pop()
                if condition == "failed":
                    evidence["observations"]["candidate"][self.workload][-1]["rss_residual_bytes"] = 2 * collector.MIB
                with (self.subTest(build_mode=build_mode, condition=condition),
                      mock.patch.object(collector, "evaluate", side_effect=[evidence, calibration, control]),
                      mock.patch.object(collector, "runtime_closure_admission")):
                    result = collector.report(directory)
                self.assertEqual(result["status"], condition)
                self.assertFalse(result["performance_qualified"])
                self.assertEqual(result["whole_change_acceptance"], "incomplete")
                expected = "failed" if condition == "failed" else "incomplete"
                self.assertEqual(result["targets"], {TARGET: expected, collector.TARGETS[1]: "not-run"})
                verdict = "normal_timing_verdict" if build_mode == "normal" else "heap_and_work_verdict"
                self.assertEqual(result[verdict], expected)
                coverage = result["workload_coverage"]
                required = [row["id"] for row in v3_manifest(build_mode)["workloads"]]
                self.assertEqual(coverage, {"required": required, "selected": [self.workload],
                                           "omitted": [row for row in required if row != self.workload],
                                           "complete": False})
                self.assertEqual(list(result["process_observations"]["candidate"]), [self.workload])
                if condition == "passed" and build_mode == "normal":
                    self.assertTrue(all(value["passed"] for value in result["benefits"][self.workload].values()))
                if condition == "failed":
                    self.assertTrue(any("rss_residual_bytes" in issue["reason"] for issue in result["issues"]))

    def test_single_workload_grading_keeps_complete_process_and_numerical_gates(self):
        calibration, control, comparison = self.cohorts()
        profile = self.freeze(calibration, control)
        for missing in ("row", "sixth", "metric"):
            changed = copy.deepcopy(calibration)
            rows = changed["observations"]["baseline"][self.workload]
            if missing == "row":
                del changed["observations"]["baseline"][self.workload]
            elif missing == "sixth":
                rows.pop()
            else:
                rows[-1]["cpu_ns"] = None
            with self.subTest(missing=missing), self.assertRaises(ValueError):
                collector.derive_envelopes(changed)
        for metric in ("p95_ns", "cpu_ns"):
            changed = copy.deepcopy(comparison)
            baseline = changed["observations"]["baseline"][self.workload][0][metric]
            for row in changed["observations"]["candidate"][self.workload]:
                row[metric] = baseline - profile["envelopes"][self.workload]["benefit_allowances"][metric]
            issues, benefits = collector.compare_evidence(changed, profile)
            self.assertFalse(benefits[self.workload][metric]["passed"])
            self.assertTrue(any(f"{metric} benefit" in issue["reason"] for issue in issues))
        control["observations"]["control"][self.workload][-1]["rss_residual_bytes"] = 2 * collector.MIB
        with self.assertRaisesRegex(ValueError, "A/A control failed"):
            collector.profile_from_v2(calibration, profile["calibration"]["path"], control,
                                      profile["control"]["path"], "reviewer", "scoped fixture", None, None)

    def test_full_corpus_bindings_order_and_fixed_limits_remain_available(self):
        for build_mode, count in (("normal", 54), ("instrumented", 57)):
            for mode, manifests, processes, cap in (("calibration", 1, 6, 21600),
                                                    ("control", 2, 12, 43200),
                                                    ("comparison", 2, 12, 43200)):
                root = self.root / f"{build_mode}-{mode}"
                root.mkdir()
                specification = v3_plan(root, mode, build_mode)
                binding = save_method_review(root, specification)
                specification["method_review"] = binding
                with self.subTest(build_mode=build_mode, mode=mode):
                    collector.validate_plan(specification)
                    self.assertEqual(set(binding), {"path", "sha256"})
                    self.assertEqual(len(specification["workloads"]), count)
                    self.assertEqual(len(specification["schedule"]), manifests + processes * count)
                    self.assertEqual(specification["execution_limits"]["campaign_timeout_seconds"], cap)
                    changed = copy.deepcopy(specification)
                    changed["workloads"].reverse()
                    changed["schedule"] = collector.schedule(mode, changed["workloads"], 3)
                    with self.assertRaisesRegex(ValueError, "workload coverage"):
                        collector.validate_plan(changed)
                    review = collector.load(binding["path"])
                    review["stage_limits"][mode]["campaign_timeout_seconds"] -= 1
                    Path(binding["path"]).write_bytes(collector.document(review))
                    with self.assertRaisesRegex(ValueError, "finite cost/bounds"):
                        collector.reviewed_method(binding["path"], collector.sha256(binding["path"]), specification)


UNCACHED_ROLE = "uncached-bgra-repeated-scan-v1"


def uncached_format_contract():
    cases = []
    for name, source, padding, roi, forced in (
        ("rgba-full", "rgba8", 0, False, False), ("bgra-full", "bgra8", 0, False, False),
        ("rgba-padded-full", "rgba8", 16, False, False), ("bgra-padded-full", "bgra8", 16, False, False),
        ("rgba-roi", "rgba8", 0, True, False), ("bgra-roi", "bgra8", 0, True, False),
        ("rgba-padded-roi", "rgba8", 16, True, False), ("bgra-padded-roi", "bgra8", 16, True, False),
        ("rgba-forced-bgra-full", "rgba8", 0, False, True),
        ("rgba-forced-bgra-roi", "rgba8", 0, True, True),
    ):
        pixels = 57_600 if roi else 2_073_600
        copied = source == "rgba8" or roi
        cases.append({"id": name, "source_format": source, "selected_format": "bgra8",
                      "row_padding_bytes": padding, "nonzero_roi": roi, "forced_bgra_public_mapping": forced,
                      "expected_matches": 1, "expected_bounds": [80, 64, 92, 74],
                      "expected_mapping_copies": int(copied),
                      "expected_mapping_copied_bytes": pixels * 4 if copied else 0,
                      "expected_bgr_conversions": 1, "expected_bgr_converted_bytes": pixels * 3})
    return {
        "cases": cases,
        "transition_cases": [
            {"id": "same-source-roi", "selected_format": "bgra8", "region": [64, 48, 384, 228],
             "expected_bounds": [80, 64, 92, 74], "mapping_copies": 1, "mapping_copied_bytes": 230400,
             "bgr_conversions": 1, "bgr_converted_bytes": 172800},
            {"id": "same-source-alternate-roi", "selected_format": "bgra8", "region": [72, 56, 376, 220],
             "expected_bounds": [152, 64, 164, 74], "mapping_copies": 1, "mapping_copied_bytes": 199424,
             "bgr_conversions": 1, "bgr_converted_bytes": 149568},
            {"id": "distinct-bgra-full", "selected_format": "bgra8", "region": [0, 0, 1920, 1080],
             "expected_bounds": [80, 64, 92, 74], "mapping_copies": 0, "mapping_copied_bytes": 0,
             "bgr_conversions": 1, "bgr_converted_bytes": 6220800},
        ],
        "lifetime_order": ["persistent-uncached", "active-uncached"],
        "persistent_checkpoint_order": ["mapping-live", "cold-result", "repeated-search", "mapping-released",
                                        "frame-released", "result-released", "final-backend-released"],
        "active_checkpoint_order": ["prepared-reader-paused", "active-reader-after-pressure", "final-reader-completed",
                                    "frames-released", "results-released", "final-backend-released"],
        "expected_per_iteration": {"backend_find_calls": 18, "bgr_conversions": 18,
                                   "bgr_converted_bytes": 57_519_168,
                                   "mapping_copies": 15, "mapping_copied_bytes": 51_809_024},
        "operation_timeout_seconds": 5, "prepared_reader_pause_timeout_seconds": 5,
        "resource_scope": "logical mapping owners and cache-retained/evicted BGR gauges; uncached native BGR scratch is not a cache gauge or total native allocation/RSS observation",
        "active_owner_order": "final production reader completes before final backend release",
    }


def uncached_manifest(build_mode="instrumented"):
    value = v3_manifest(build_mode)
    value["apparatus"]["accounting_contract"] = UNCACHED_ROLE
    for row in value["workloads"]:
        if row["id"] == "contract-roi-format-transition":
            row["format_contract"] = uncached_format_contract()
    return value


def uncached_payload(workload, build_mode="instrumented"):
    value = v3_payload(workload, build_mode, smoke=True)
    declaration = uncached_manifest(build_mode)
    value["apparatus"] = declaration["apparatus"]
    row = value["workloads"][0]
    if build_mode == "normal":
        return value
    metrics = row["metrics"]
    metrics["native_allocation_scope"] = "cache-payload gauges and process RSS, not total native allocation tracking"
    for name in ("bgr_retained_bytes", "bgr_retained_peak_bytes", "bgr_retained_after_drop_bytes",
                 "bgr_evicted_bytes", "bgr_evicted_peak_bytes", "bgr_evicted_after_drop_bytes"):
        metrics[name] = 0
    if workload in collector.SESSION_DEFINITIONS:
        case = row["session_contract"]["case"]
        calls, scans = {
            "retained-hit": (1, 2), "retained-miss": (1, 1), "patch64": (1, 2),
            "group": (8, 16), "distinct-group": (8, 16), "owner-group": (8, 16),
            "turnover": (4, 6), "query-match": (3, 5), "query-deadline": (1, 1),
            "lifecycle-session": (1, 2), "lifecycle-independent": (4, 8),
        }[case]
        left, top, right, bottom = row["effective_regions"][0]
        pixels = (right - left) * (bottom - top)
        copied = "-roi" in workload or row["session_contract"]["source_format"] == "rgba8"
        copies = calls * int(copied) + int(case in ("owner-group", "lifecycle-session"))
        for unit in row["unit_observations"]:
            unit["work"] = {"accounting_contract": UNCACHED_ROLE,
                            "backend_find_calls": calls, "backend_find_completions": calls,
                            "backend_find_failures": 0, "bgr_conversions": calls,
                            "bgr_converted_bytes": calls * pixels * 3, "mapping_copies": copies,
                            "mapping_copied_bytes": copies * pixels * 4, "correct": True}
            if case == "lifecycle-independent":
                unit["lifecycle"].update(a_clone_reused=False, b_independent=True, b_reused=False)
            for point in unit["checkpoints"]:
                point.update(bgr_retained_bytes=0, bgr_evicted_bytes=0)
        for name in ("backend_find_calls", "backend_find_failures", "bgr_conversions", "bgr_converted_bytes",
                     "mapping_copies", "mapping_copied_bytes"):
            metrics[name] = sum(unit["work"][name] for unit in row["unit_observations"])
        patch_width, patch_height = row["session_contract"]["patch_extent"]
        offsets = (right - left - patch_width + 1) * (bottom - top - patch_height + 1)
        metrics.update(score_copied_bytes=4 * offsets * calls * 4,
                       score_offsets_scanned=4 * offsets * scans,
                       logical_mapped_bytes=4 * calls * pixels * 4)
    elif workload == "contract-roi-format-transition":
        contract = uncached_format_contract()
        row["format_contract"] = contract
        for observed in row["format_observations"]:
            for case, expected in zip(observed["cases"], contract["cases"]):
                copied = expected["expected_mapping_copies"] != 0
                case.update(selected_format="bgra8", mapping_shared=not copied,
                            mapped_stride=1280 if expected["nonzero_roi"] else
                            7680 if copied else case["source_stride"])
                case["work"].update(mapping_copies=expected["expected_mapping_copies"],
                                    mapping_copied_bytes=expected["expected_mapping_copied_bytes"])
            for transition in observed["transitions"]:
                transition["selected_format"] = "bgra8"
            for lifetime, name, order in zip(
                observed["lifetimes"], contract["lifetime_order"],
                (contract["persistent_checkpoint_order"], contract["active_checkpoint_order"]),
            ):
                lifetime["id"] = name
                lifetime["work"] = (
                    {"mapping_copies": 2, "mapping_copied_bytes": 460800,
                     "bgr_conversions": 2, "bgr_converted_bytes": 345600}
                    if name == "persistent-uncached" else
                    {"mapping_copies": 3, "mapping_copied_bytes": 24883200,
                     "bgr_conversions": 3, "bgr_converted_bytes": 18662400})
                for point, stage in zip(lifetime["checkpoints"], order):
                    point.update(checkpoint=stage, cache_held_bgr_bytes=0, active_evicted_bgr_bytes=0)
            observed["work"] = {key: item for key, item in contract["expected_per_iteration"].items()
                                if key != "backend_find_calls"}
        metrics.update({key: item * 4 for key, item in contract["expected_per_iteration"].items()})
        metrics.update(score_copied_bytes=301_903_648, score_offsets_scanned=75_475_912,
                       logical_mapped_bytes=306_838_016)
    elif workload.startswith("selection-"):
        offsets, scans = {
            "selection-k1-large": (8_294_400, 1),
            "selection-k16-dense1080p-keepall": (2_073_600, 16),
            "selection-k16-dense1080p-dropoverlapping": (2_073_600, 16),
            "selection-k256-small-dense-tied": (6_144, 256),
            "selection-k256-large-sparse": (8_294_400, 4),
        }[workload]
        metrics.update(score_copied_bytes=offsets * 16, score_offsets_scanned=offsets * scans * 4,
                       backend_find_calls=0, bgr_conversions=0, bgr_converted_bytes=0,
                       mapping_copies=0, mapping_copied_bytes=0, logical_mapped_bytes=0)
    elif workload in ("watch-analysis-always", "watch-exact-rgba"):
        metrics.update(backend_find_calls=16, bgr_conversions=16, bgr_converted_bytes=36_864,
                       mapping_copies=16, mapping_copied_bytes=49_152,
                       score_copied_bytes=20_160, score_offsets_scanned=5_040,
                       logical_mapped_bytes=49_152)
    else:
        calls, copies, width, height = {
            "single-small96x64": (4, 4, 96, 64), "single-1080p": (4, 4, 1920, 1080),
            "single-4k": (4, 4, 3840, 2160), "batch8-cold-full1080p": (32, 32, 1920, 1080),
            "batch8-cold-roi320x180": (32, 32, 320, 180),
            "batch8-live-owner-roi320x180": (32, 36, 320, 180),
            "batch8-warm-full1080p": (40, 40, 1920, 1080),
            "concurrent2-cold-full1080p": (8, 8, 1920, 1080),
            "contract-different-frame": (8, 8, 1920, 1080), "contract-equal-stamp": (8, 8, 1920, 1080),
            "contract-expired-mapping": (8, 12, 320, 180), "contract-eviction": (16, 16, 1920, 1080),
            "contract-oversized-bgr": (4, 4, 4096, 2732),
        }[workload]
        metrics.update(backend_find_calls=calls, bgr_conversions=calls,
                       bgr_converted_bytes=calls * width * height * 3,
                       mapping_copies=copies, mapping_copied_bytes=copies * width * height * 4,
                       score_copied_bytes=calls * (width - 11) * (height - 9) * 4,
                       score_offsets_scanned=calls * (width - 11) * (height - 9),
                       logical_mapped_bytes=calls * width * height * 4)
    return value


class BaselineRoleAdmissionTests(unittest.TestCase):
    def parse(self, value, workload, mode="instrumented"):
        frozen = collector.manifest_identity(uncached_manifest(mode), TARGET, "4.12.0", 3, mode)
        return collector.observations(value, frozen, workload, True)

    def test_both_roles_share_only_validated_logical_semantics(self):
        for mode in ("normal", "instrumented"):
            baseline = collector.manifest_identity(uncached_manifest(mode), TARGET, "4.12.0", 3, mode)
            candidate = collector.manifest_identity(v3_manifest(mode), TARGET, "4.12.0", 3, mode)
            self.assertNotEqual(baseline, candidate)
            self.assertEqual(collector.common_manifest(baseline), collector.common_manifest(candidate))
            for workload, _, _, _ in V3_ROWS:
                with self.subTest(mode=mode, workload=workload):
                    self.assertEqual(self.parse(uncached_payload(workload, mode), workload, mode)["unavailable"], [])
        for workload in collector.DEFINITIONS:
            with self.subTest(workload=workload):
                self.parse(uncached_payload(workload), workload)

    def test_declared_expectations_cannot_override_physical_role(self):
        for change in ("wrong-role", "case-copy", "selected-format", "total", "active-stage", "lifetime-kind"):
            declaration = uncached_manifest()
            if change == "wrong-role":
                declaration["apparatus"]["accounting_contract"] = "prepared-cache-row-winner-v1"
            else:
                direct = next(row for row in declaration["workloads"] if row["id"] == "contract-roi-format-transition")["format_contract"]
                if change == "case-copy":
                    direct["cases"][0]["expected_mapping_copies"] = 0
                elif change == "selected-format":
                    direct["cases"][0]["selected_format"] = "rgba8"
                elif change == "total":
                    direct["expected_per_iteration"]["bgr_conversions"] = 17
                elif change == "active-stage":
                    direct["active_checkpoint_order"][1] = "active-image-evicted"
                else:
                    direct["lifetime_order"][1] = "active-evicted"
            with self.subTest(change=change), self.assertRaises(ValueError):
                collector.manifest_identity(declaration, TARGET, "4.12.0", 3, "instrumented")

    def test_role_confusion_cache_hits_copies_and_selector_work_fail_closed(self):
        workload = "session-retained-rgba-full-hit"
        for change in ("role", "hit-conversion", "mapping", "gauge", "score-copy", "scan"):
            value = uncached_payload(workload)
            row = value["workloads"][0]
            if change == "role":
                value["apparatus"]["accounting_contract"] = "prepared-cache-row-winner-v1"
            elif change == "hit-conversion":
                row["unit_observations"][1]["work"]["bgr_conversions"] = 0
            elif change == "mapping":
                row["unit_observations"][0]["work"]["mapping_copies"] = 0
            elif change == "gauge":
                row["metrics"]["bgr_retained_peak_bytes"] = 1
            elif change == "score-copy":
                row["metrics"]["score_copied_bytes"] = 0
            else:
                row["metrics"]["score_offsets_scanned"] -= 1
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.parse(value, workload)
        workload = "selection-k256-large-sparse"
        value = uncached_payload(workload)
        value["workloads"][0]["metrics"]["score_offsets_scanned"] = 3 * 8_294_400 * 4
        with self.assertRaises(ValueError):
            self.parse(value, workload)

    def test_uncached_component_cold_warm_owner_and_format_accounting_cannot_be_relabeled(self):
        for workload, field in (
            ("batch8-cold-full1080p", "bgr_conversions"),
            ("batch8-warm-full1080p", "backend_find_calls"),
            ("batch8-live-owner-roi320x180", "mapping_copies"),
            ("contract-expired-mapping", "mapping_copied_bytes"),
            ("watch-exact-rgba", "mapping_copies"),
            ("contract-roi-format-transition", "score_copied_bytes"),
        ):
            value = uncached_payload(workload)
            value["workloads"][0]["metrics"][field] -= 1
            with self.subTest(workload=workload, field=field), self.assertRaises(ValueError):
                self.parse(value, workload)

    def test_uncached_mapped_views_include_repeated_searches_and_shared_padding(self):
        for workload in (
            "single-1080p", "batch8-warm-full1080p", "batch8-live-owner-roi320x180",
            "session-retained-rgba-full-hit", "selection-k256-large-sparse",
            "watch-exact-rgba", "contract-roi-format-transition",
        ):
            value = uncached_payload(workload)
            metrics = value["workloads"][0]["metrics"]
            expected = metrics["logical_mapped_bytes"]
            for invalid in {0, expected + 1} - {expected}:
                metrics["logical_mapped_bytes"] = invalid
                with self.subTest(workload=workload, invalid=invalid), self.assertRaises(ValueError):
                    self.parse(value, workload)
        padded = uncached_payload("contract-roi-format-transition")
        padded["workloads"][0]["metrics"]["logical_mapped_bytes"] -= 4 * 1080 * 16
        with self.assertRaises(ValueError):
            self.parse(padded, "contract-roi-format-transition")

    def test_retained_smoke_admits_baseline_and_preserves_failed_role_raw_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            for broken in (False, True):
                root = Path(temporary) / ("failed" if broken else "accepted")
                root.mkdir()
                (root / "started").mkdir()
                (root / "attempts").mkdir()
                workload, mode = "session-retained-rgba-full-hit", "instrumented"
                declaration = uncached_manifest(mode)
                specification = v3_plan(root.parent, "smoke", mode)
                binary = specification["binaries"]["baseline"]
                binary["build"].update(
                    apparatus=copy.deepcopy(declaration["apparatus"]),
                    benchmark_manifest=collector.manifest_identity(declaration, TARGET, "4.12.0", 3, mode))
                binary["build"]["provenance_commands"][0]["record"] = process_record(declaration)
                specification.update(workloads=[workload], schedule=collector.schedule("smoke", [workload], 3))
                specification["execution_limits"] = collector.execution_limits("smoke", 2)
                collector.save(root / "plan.json", specification)
                raw = {}
                for index, task in enumerate(specification["schedule"]):
                    attempt = {"task": task, "argv": collector.command(binary, task, "smoke", 3),
                               "plan_sha256": collector.sha256(root / "plan.json"), "started_unix_ns": index + 1,
                               "campaign_elapsed_before": index, "output_limit_bytes": 8 * 1024 * 1024,
                               "cleanup_seconds": 5, "runtime_binding_seconds": 0}
                    filename = root / "attempts" / f"{task['id']}.json"
                    collector.save(root / "started" / filename.name, attempt)
                    value = copy.deepcopy(declaration) if index == 0 else uncached_payload(workload)
                    if broken and index == 1:
                        value["apparatus"]["accounting_contract"] = "prepared-cache-row-winner-v1"
                    attempt.update(identity_before=binary, identity_after=binary, identity_error=None,
                                   campaign_elapsed_after=index + 0.1, record=process_record(value), timeout_seconds=90)
                    collector.save(filename, attempt)
                    raw[filename] = filename.read_bytes()
                evidence = collector.report(root)
                self.assertEqual(bool(evidence["issues"]), broken)
                for filename, content in raw.items():
                    self.assertEqual(filename.read_bytes(), content)
                if not broken:
                    self.assertIn(workload, evidence["process_observations"]["baseline"])

    def test_lifecycle_and_direct_active_work_remain_real_and_role_specific(self):
        for workload, change in (
            ("lifecycle-independent-rgba-4k", "reuse"),
            ("lifecycle-session-rgba-roi", "cache"),
            ("contract-roi-format-transition", "active"),
            ("contract-roi-format-transition", "shared"),
        ):
            value = uncached_payload(workload)
            row = value["workloads"][0]
            if change == "reuse":
                row["unit_observations"][0]["lifecycle"]["a_clone_reused"] = True
            elif change == "cache":
                row["unit_observations"][0]["checkpoints"][1]["bgr_retained_bytes"] = 172800
            elif change == "active":
                row["format_observations"][0]["lifetimes"][1]["checkpoints"][0]["active_backend_finds"] = 0
            else:
                row["format_observations"][0]["cases"][0]["mapping_shared"] = True
            with self.subTest(workload=workload, change=change), self.assertRaises(ValueError):
                self.parse(value, workload)

    def test_common_comparison_never_erases_logical_changes(self):
        baseline = collector.manifest_identity(uncached_manifest(), TARGET, "4.12.0", 3, "instrumented")
        for field, value in (("min_score", 0.5), ("timing_boundary", "different public operation")):
            changed = v3_manifest("instrumented")
            changed["workloads"][0][field] = value
            changed = collector.manifest_identity(changed, TARGET, "4.12.0", 3, "instrumented")
            self.assertNotEqual(collector.common_manifest(baseline), collector.common_manifest(changed))
        changed = copy.deepcopy(baseline)
        direct = next(row for row in changed["workloads"] if row["id"] == "contract-roi-format-transition")
        direct["format_contract"]["cases"][0]["expected_bounds"][0] = 81
        with self.assertRaises(ValueError):
            collector.common_manifest(changed)


class ExportedSourceAttestationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.source = self.root / "export"
        self.source.mkdir()
        self.binary = self.root / "binary"
        self.native = self.root / "libopencv_core.fixture"
        self.binary.write_bytes(b"real explicit binary fixture")
        self.native.write_bytes(b"native library fixture")
        self.revision, self.tree = "a" * 40, "c" * 40
        self.declaration = uncached_manifest("normal")
        self.declaration["target"] = NATIVE_TARGET
        self.declaration["loaded_native_runtime"] = {
            "method": "K32EnumProcessModules-GetModuleFileNameW" if os.name == "nt" else RUNTIME["method"],
            "images": [{"path": str(self.native), "sha256": collector.sha256(self.native)}]}
        self.contents = {"Cargo.lock": b"locked dependencies\n", "Cargo.toml": b"[workspace]\n",
                         "crates/backend/src/physical.rs": b"uncached historical backend fixture\n",
                         **{path: f"compiled source fixture {index}\n".encode()
                            for index, path in enumerate(V3_APPARATUS_SOURCES)}}
        entries = []
        for path, content in self.contents.items():
            target = self.source / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
            blob = hashlib.sha1(f"blob {len(content)}\0".encode() + content).hexdigest()
            entries.append(f"100644 blob {blob}\t{path}\0")
            if path in V3_APPARATUS_SOURCES:
                self.declaration["apparatus"]["source_files"][path] = hashlib.sha256(content).hexdigest()
        entries.append("120000 blob 681311eb9cf453d0faddf3aacaec7357e97ba8e9\tAGENTS.md\0")
        self.listing = "".join(entries)
        self.output = self.root / "metadata.json"
        self.args = collector.parser().parse_args([
            "attest", "--protocol-version", "3", "--build-mode", "normal", "--target", NATIVE_TARGET, "--binary", str(self.binary),
            "--build-argv-json", '["cargo","bench","--locked","--no-run"]', "--output", str(self.output),
            "--source-root", str(self.source), "--source-revision", self.revision])

    def run_attest(self, *, outputs=None, hook=None, record_hook=None):
        outputs = outputs if outputs is not None else [
            self.declaration, self.revision + "\n", self.tree + "\n", self.listing, "rustc fixture\n"]
        count = 0

        def process(argv, **kwargs):
            nonlocal count
            index = count
            count += 1
            if hook is not None:
                hook(index, argv, kwargs)
            record = process_record(outputs[index])
            if record_hook is not None:
                record_hook(index, record)
            return record

        with (mock.patch.object(collector, "ROOT", self.root),
              mock.patch.object(collector, "host_identity", return_value={"target": NATIVE_TARGET}),
              mock.patch.object(collector.shutil, "which", return_value=sys.executable),
              mock.patch.object(collector, "run_process", side_effect=process) as launch):
            result = collector.attest(self.args)
        return result, launch

    def assert_failed_receipt(self):
        self.assertFalse(self.output.exists())
        retained = collector.load(str(self.output) + ".probe.json")
        self.assertIsNotNone(retained["error"])
        self.assertEqual(retained["source_binding"]["variant"], "exported-git-tree-v1")
        return retained

    def test_exact_export_binds_actual_commit_tree_lock_and_compiled_sources(self):
        result, launch = self.run_attest()
        self.assertEqual(result["sha256"], collector.sha256(self.output))
        build = collector.load(self.output)
        self.assertEqual(launch.call_count, 5)
        self.assertEqual(build["source_revision"], self.revision)
        self.assertEqual(build["source_tree"], self.tree)
        self.assertNotIn("source_status_porcelain", build)
        binding = build["source_binding"]
        self.assertEqual(binding["before"], binding["after"])
        self.assertEqual(binding["before"]["file_mode_method"],
                         "windows-regular-file-bytes" if os.name == "nt" else "posix-executable-bits")
        self.assertEqual(set(binding["before"]["files"]), set(self.contents))
        self.assertEqual(binding["before"]["omitted_context_links"], {
            "AGENTS.md": {"mode": "120000", "git_object": "681311eb9cf453d0faddf3aacaec7357e97ba8e9"}})
        self.assertEqual(build["cargo_lock_sha256"], hashlib.sha256(self.contents["Cargo.lock"]).hexdigest())
        for probe, call in zip(build["provenance_commands"], launch.call_args_list):
            self.assertEqual(probe["argv"], call.args[0])
            self.assertEqual(probe["cwd"], str(call.kwargs["cwd"]))
            self.assertLessEqual(call.kwargs["timeout_seconds"], 90)
            self.assertEqual(call.kwargs["output_limit_bytes"], 8 * 1024 * 1024)
        collector.validate_v2_build({"path": str(self.binary), "build": build}, NATIVE_TARGET, "normal", 3)
        original, probe = self.output.read_bytes(), Path(str(self.output) + ".probe.json").read_bytes()
        with mock.patch.object(collector, "run_process") as blocked, self.assertRaises(ValueError):
            collector.attest(self.args)
        blocked.assert_not_called()
        self.assertEqual(self.output.read_bytes(), original)
        self.assertEqual(Path(str(self.output) + ".probe.json").read_bytes(), probe)

    def test_path_and_descriptor_change_times_have_independent_stable_baselines(self):
        fstat = os.fstat

        def descriptor_stat(fd):
            info = fstat(fd)
            fields = {name: getattr(info, name) for name in dir(info) if name.startswith("st_")}
            fields["st_ctime_ns"] += 1_000_000
            return SimpleNamespace(**fields)

        with mock.patch.object(collector.os, "fstat", side_effect=descriptor_stat):
            self.run_attest()
        binding = collector.load(self.output)["source_binding"]
        self.assertEqual(binding["before"], binding["after"])
        for path, content in self.contents.items():
            self.assertEqual(binding["after"]["files"][path]["sha256"], hashlib.sha256(content).hexdigest())

    def test_change_times_within_either_metadata_api_still_refuse_drift(self):
        expected, omissions = collector.exported_tree(self.listing)
        victim = self.source / "Cargo.lock"
        stat_call, fstat_call = os.stat, os.fstat
        victim_info = stat_call(victim)
        for api, original in (("stat", stat_call), ("fstat", fstat_call)):
            with self.subTest(api=api):
                calls = 0

                def changed(*args, **kwargs):
                    nonlocal calls
                    info = original(*args, **kwargs)
                    if (info.st_dev, info.st_ino) != (victim_info.st_dev, victim_info.st_ino):
                        return info
                    calls += 1
                    if calls != 2:
                        return info
                    fields = {name: getattr(info, name) for name in dir(info) if name.startswith("st_")}
                    fields["st_ctime_ns"] += 1_000_000
                    return SimpleNamespace(**fields)

                receipt = {}
                with (mock.patch.object(collector.os, api, side_effect=changed),
                      self.assertRaisesRegex(ValueError, "source changed while hashing: Cargo.lock")):
                    collector.verify_export(self.source, expected, omissions, time.monotonic() + 60, receipt)
                self.assertFalse(receipt["complete"])
                self.assertNotIn("Cargo.lock", receipt["files"])

    def test_directory_cache_without_file_identity_still_binds_real_source_bytes(self):
        scandir = os.scandir

        @contextlib.contextmanager
        def cached_entries(directory):
            with scandir(directory) as entries:
                cached = []
                for entry in entries:
                    fields = list(entry.stat(follow_symlinks=False))
                    fields[1:4] = [0, 0, 0]
                    cached.append(SimpleNamespace(
                        name=entry.name, path=entry.path,
                        stat=mock.Mock(return_value=os.stat_result(fields))))
                yield iter(cached)

        with mock.patch.object(collector.os, "scandir", side_effect=cached_entries):
            self.run_attest()
        binding = collector.load(self.output)["source_binding"]
        self.assertEqual(binding["before"], binding["after"])
        for path, content in self.contents.items():
            self.assertEqual(binding["before"]["files"][path]["sha256"], hashlib.sha256(content).hexdigest())

    def test_paired_options_fail_without_launch_and_keep_failure(self):
        for field in ("source_root", "source_revision"):
            with self.subTest(field=field):
                self.args.output = str(self.root / f"unpaired-{field}.json")
                saved = getattr(self.args, field)
                setattr(self.args, field, None)
                with mock.patch.object(collector, "run_process") as launch, self.assertRaises(ValueError):
                    collector.attest(self.args)
                launch.assert_not_called()
                self.assertFalse(Path(self.args.output).exists())
                self.assertEqual(collector.load(self.args.output + ".probe.json")["commands"], [])
                setattr(self.args, field, saved)

    def test_wrong_missing_changed_extra_sources_and_lock_fail_with_retention(self):
        for change in ("wrong-root", "missing", "changed", "extra", "lock", "extra-directory"):
            with self.subTest(change=change):
                self.args.output = str(self.root / f"{change}.json")
                victim = self.source / "crates/backend/src/physical.rs"
                original = victim.read_bytes()
                lock = self.source / "Cargo.lock"
                if change == "wrong-root":
                    self.args.source_root = str(self.root)
                elif change == "missing":
                    victim.unlink()
                elif change == "changed":
                    victim.write_bytes(b"changed backend")
                elif change == "extra":
                    (self.source / "extra.rs").write_bytes(b"extra source")
                elif change == "lock":
                    lock.write_bytes(b"other dependencies")
                else:
                    (self.source / "extra").mkdir()
                with self.assertRaises((ValueError, OSError)):
                    self.run_attest()
                retained = collector.load(self.args.output + ".probe.json")
                self.assertFalse(Path(self.args.output).exists())
                self.assertEqual(len(retained["commands"]), 4)
                self.assertFalse(retained["source_binding"]["before"].get("complete", False))
                victim.write_bytes(original)
                lock.write_bytes(self.contents["Cargo.lock"])
                self.args.source_root = str(self.source)
                if change == "extra":
                    (self.source / "extra.rs").unlink()
                if change == "extra-directory":
                    (self.source / "extra").rmdir()

    def test_changed_compiled_apparatus_and_wrong_revision_never_attest(self):
        for change in ("compiled", "revision", "tree"):
            self.args.output = str(self.root / f"bad-{change}.json")
            outputs = [copy.deepcopy(self.declaration), self.revision + "\n",
                       self.tree + "\n", self.listing, "rustc fixture\n"]
            if change == "compiled":
                outputs[0]["apparatus"]["source_files"][V3_APPARATUS_SOURCES[0]] = "0" * 64
            elif change == "revision":
                outputs[1] = "b" * 40 + "\n"
            else:
                outputs[2] = "not a tree\n"
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.run_attest(outputs=outputs)
            retained = collector.load(self.args.output + ".probe.json")
            self.assertFalse(Path(self.args.output).exists())
            self.assertIn("error", retained)

    def test_export_and_binary_mutation_between_probes_fail_with_both_passes(self):
        for change in ("source", "binary"):
            self.args.output = str(self.root / f"mutation-{change}.json")
            victim = self.source / "Cargo.lock" if change == "source" else self.binary
            content = victim.read_bytes()

            def mutate(index, _argv, _kwargs):
                if index == 4:
                    victim.write_bytes(b"changed during rustc probe")

            with self.subTest(change=change), self.assertRaises(ValueError):
                self.run_attest(hook=mutate)
            retained = collector.load(self.args.output + ".probe.json")
            self.assertFalse(Path(self.args.output).exists())
            self.assertEqual(len(retained["commands"]), 5)
            self.assertTrue(retained["source_binding"]["before"]["complete"])
            self.assertEqual(retained["commands"][-1]["record"]["stdout"], "rustc fixture\n")
            victim.write_bytes(content)

    def test_links_unsafe_tree_paths_and_unapproved_omissions_are_rejected(self):
        for entry in (
            "100644 blob " + "a" * 40 + "\t../escape.rs\0",
            "100644 blob " + "a" * 40 + "\tC:/escape.rs\0",
            "100644 blob " + "a" * 40 + "\tcrates\\escape.rs\0",
            "120000 blob " + "a" * 40 + "\tbuild.rs\0",
            "120000 blob " + "a" * 40 + "\tAGENTS.md\0",
            "160000 commit " + "a" * 40 + "\tdependency\0",
        ):
            self.args.output = str(self.root / f"unsafe-{hashlib.sha256(entry.encode()).hexdigest()}.json")
            outputs = [self.declaration, self.revision + "\n", self.tree + "\n",
                       self.listing + entry, "rustc fixture\n"]
            with self.subTest(entry=entry), self.assertRaises(ValueError):
                self.run_attest(outputs=outputs)
            self.assertFalse(Path(self.args.output).exists())
            self.assertEqual(len(collector.load(self.args.output + ".probe.json")["commands"]), 4)
        if os.name != "nt":
            victim = self.source / "Cargo.lock"
            victim.unlink()
            victim.symlink_to(self.native)
            self.args.output = str(self.root / "source-link.json")
            with self.assertRaisesRegex(ValueError, "link"):
                self.run_attest()
            self.assertFalse(Path(self.args.output).exists())
            self.assertIsNotNone(collector.load(self.args.output + ".probe.json")["error"])

    def test_source_deadline_and_launch_exception_retain_actual_completed_probes(self):
        def fail(index, _argv, _kwargs):
            if index == 4:
                raise OSError("compiler identity probe could not launch")

        with self.assertRaisesRegex(OSError, "could not launch"):
            self.run_attest(hook=fail)
        retained = self.assert_failed_receipt()
        self.assertEqual(len(retained["commands"]), 5)
        self.assertIsNone(retained["commands"][-1]["record"])
        self.assertTrue(retained["source_binding"]["before"]["complete"])
        self.args.output = str(self.root / "deadline.json")
        with (mock.patch.object(collector, "SOURCE_LIMITS", {**collector.SOURCE_LIMITS, "verification_seconds": 0}),
              self.assertRaisesRegex(ValueError, "deadline")):
            self.run_attest()
        retained = collector.load(self.args.output + ".probe.json")
        self.assertEqual(len(retained["commands"]), 4)
        self.assertFalse(retained["source_binding"]["before"]["complete"])

    def test_failed_or_truncated_tree_probe_retains_exact_partial_output_without_source_claim(self):
        for field, value in (("exit_code", 1), ("timed_out", True), ("output_limited", True), ("cleanup_ok", False)):
            self.args.output = str(self.root / f"failed-tree-{field}.json")

            def fail(index, record):
                if index == 3:
                    record[field] = value
                    record["stdout"] = "retained partial Git tree output"

            with self.subTest(field=field), self.assertRaises(ValueError):
                self.run_attest(record_hook=fail)
            self.assertFalse(Path(self.args.output).exists())
            retained = collector.load(self.args.output + ".probe.json")
            self.assertEqual(len(retained["commands"]), 4)
            self.assertEqual(retained["commands"][-1]["record"]["stdout"], "retained partial Git tree output")
            self.assertFalse(retained["source_binding"]["before"].get("complete", False))

    def test_source_traversal_and_byte_limits_preserve_partial_verification(self):
        for field, limit in (("entries", 1), ("file_bytes", 1), ("total_bytes", 1)):
            self.args.output = str(self.root / f"limit-{field}.json")
            with (mock.patch.object(collector, "SOURCE_LIMITS", {**collector.SOURCE_LIMITS, field: limit}),
                  self.subTest(field=field), self.assertRaises(ValueError)):
                self.run_attest()
            retained = collector.load(self.args.output + ".probe.json")
            self.assertFalse(Path(self.args.output).exists())
            self.assertEqual(len(retained["commands"]), 4)
            self.assertIsNotNone(retained["error"])

    def test_hardlinked_export_cannot_attest_shared_source(self):
        os.link(self.source / "Cargo.lock", self.root / "outside-source-link")
        with self.assertRaisesRegex(ValueError, "hard-linked"):
            self.run_attest()
        retained = self.assert_failed_receipt()
        self.assertFalse(retained["source_binding"]["before"]["complete"])

    @unittest.skipIf(os.name == "nt", "POSIX executable-bit and link observations")
    def test_executable_mode_and_materialized_context_link_are_rejected(self):
        victim = self.source / "Cargo.lock"
        victim.chmod(0o755)
        with self.assertRaisesRegex(ValueError, "differs"):
            self.run_attest()
        self.assert_failed_receipt()
        victim.chmod(0o644)
        self.args.output = str(self.root / "materialized-context.json")
        (self.source / "AGENTS.md").symlink_to("CLAUDE.md")
        with self.assertRaisesRegex(ValueError, "link"):
            self.run_attest()
        self.assertFalse(Path(self.args.output).exists())
        self.assertIsNotNone(collector.load(self.args.output + ".probe.json")["error"])

    def test_retained_source_identity_cannot_be_relabelled_as_checkout_or_other_role(self):
        self.run_attest()
        build = collector.load(self.output)
        for change in ("variant", "status", "tree", "source-file", "lock", "role"):
            value = copy.deepcopy(build)
            if change == "variant":
                value["source_binding"]["variant"] = "current-checkout"
            elif change == "status":
                value["source_status_porcelain"] = ""
            elif change == "tree":
                value["source_tree"] = "f" * 40
            elif change == "source-file":
                value["source_binding"]["after"]["files"]["Cargo.toml"]["sha256"] = "f" * 64
            elif change == "lock":
                value["cargo_lock_sha256"] = "f" * 64
            else:
                value["apparatus"]["accounting_contract"] = "prepared-cache-row-winner-v1"
            with self.subTest(change=change), self.assertRaises(ValueError):
                collector.validate_v2_build({"path": str(self.binary), "build": value}, NATIVE_TARGET, "normal", 3)


@unittest.skipIf(sys.platform == "win32", "retained Mach-O fixture uses POSIX image paths")
class ProtocolThreeRuntimePreflightTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def prepare(self, build_mode="normal", file_count=14):
        from tools.tests import test_macos_image_observer as native
        root = self.root / build_mode
        root.mkdir()
        self.run = native.SyntheticRun(root)
        self.run.lib = self.run.lib.rename(root / "libopencv_core.fixture.dylib")
        self.build_mode = build_mode
        self.runtime = {"method": RUNTIME["method"], "images": [
            {"path": str(self.run.lib), "sha256": collector.sha256(self.run.lib)}]}
        self.declaration = v3_manifest(build_mode)
        self.declaration["loaded_native_runtime"] = self.runtime
        self.declaration.update(timing_boundary_protocol="mado-component-timing-v1", timing_boundaries_enabled=False)
        contents = {"Cargo.lock": b"locked fixture dependencies\n", "Cargo.toml": b"[workspace]\n",
                    **{path: f"compiled apparatus fixture {index}\n".encode()
                       for index, path in enumerate(V3_APPARATUS_SOURCES)}}
        for index in range(file_count - len(contents)):
            path = (f"crates/adapters/fixture-backend-{index // 32:02d}/src/recognition/"
                    f"template_matching_contract_{index:04d}.rs")
            contents[path] = f"pub const FIXTURE_CASE: usize = {index};\n".encode()
        source = root / "export"
        source.mkdir()
        expected, listing = {}, []
        for path, content in contents.items():
            target = source / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
            blob = hashlib.sha1(f"blob {len(content)}\0".encode() + content).hexdigest()
            expected[path] = {"mode": "100644", "git_object": blob}
            listing.append(f"100644 blob {blob}\t{path}\0")
            if path in V3_APPARATUS_SOURCES:
                self.declaration["apparatus"]["source_files"][path] = hashlib.sha256(content).hexdigest()
        omissions = {"AGENTS.md": {"mode": "120000", "git_object": "681311eb9cf453d0faddf3aacaec7357e97ba8e9"}}
        listing.append("120000 blob 681311eb9cf453d0faddf3aacaec7357e97ba8e9\tAGENTS.md\0")
        before = collector.verify_export(source, expected, omissions, time.monotonic() + 60, {})
        after = collector.verify_export(source, expected, omissions, time.monotonic() + 60, {})
        build = v3_identity(root, build_mode)["build"]
        del build["source_status_porcelain"]
        build.update(binary_sha256=collector.sha256(self.run.main), dependencies=self.runtime["images"],
                     cargo_lock_sha256=hashlib.sha256(contents["Cargo.lock"]).hexdigest(),
                     apparatus=copy.deepcopy(self.declaration["apparatus"]),
                     benchmark_manifest=collector.manifest_identity(
                         self.declaration, TARGET, "4.12.0", 3, build_mode),
                     source_binding={"variant": "exported-git-tree-v1", "repository": str(root),
                                     "source_root": str(source), "limits": dict(collector.SOURCE_LIMITS),
                                     "before": before, "after": after})
        revision = build["source_revision"]
        probes = [
            ([str(self.run.main), "--protocol-version", "3", "--list"], self.declaration),
            (["/git", "--no-replace-objects", "rev-parse", "--verify", revision + "^{commit}"], revision + "\n"),
            (["/git", "--no-replace-objects", "rev-parse", "--verify", revision + "^{tree}"], build["source_tree"] + "\n"),
            (["/git", "--no-replace-objects", "ls-tree", "-r", "-z", "--full-tree", revision], "".join(listing)),
            (["/rustc", "-vV"], build["rustc"]),
        ]
        build["provenance_commands"] = [
            {"argv": argv, "cwd": str(root), "record": process_record(value)} for argv, value in probes]
        self.metadata = root / "base.json"
        collector.save(self.metadata, build)
        self.identity = collector.binary_identity(self.run.main, self.metadata, TARGET)
        self.launched = []

    def discover(self, argv, **options):
        self.launched.append(argv)
        value = (copy.deepcopy(self.declaration) if "--list" in argv else
                 v3_payload(argv[argv.index("--workload") + 1], self.build_mode, smoke=True))
        value["loaded_native_runtime"] = self.runtime
        value["apparatus"] = copy.deepcopy(self.declaration["apparatus"])
        value.update(timing_boundary_protocol="mado-component-timing-v1",
                     timing_boundaries_enabled="--timing-boundaries" in argv)
        from tools.tests.test_macos_image_observer import timing_row
        log = self.run.log()
        if "--list" not in argv:
            log += timing_row(argv[argv.index("--workload") + 1])
        receipt = self.run.discovery(log, stdout=json.dumps(value))
        receipt["duration_seconds"] = 0.2
        return receipt

    def preflight(self, name, discover=None):
        args = collector.parser().parse_args([
            "runtime-preflight", "--protocol-version", "3", "--build-mode", self.build_mode,
            "--target", TARGET, "--binary", str(self.run.main), "--build-metadata", str(self.metadata),
            "--output", str(self.root / name)])
        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "environment", return_value={"DYLD_LIBRARY_PATH": "/opt/lib"}),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector.macos_images, "discover", side_effect=discover or self.discover)):
            return collector.runtime_preflight(args)

    def retained_identity(self, bundle, name):
        path = self.root / f"{name}.json"
        collector.save(path, bundle)
        identity = copy.deepcopy(self.identity)
        identity["build"]["runtime_image_preflight"] = {"path": str(path), "sha256": collector.sha256(path)}
        return identity

    def test_full_source_export_preflight_and_offline_admission_fit_fixed_bundle_limit(self):
        for mode, count in (("normal", 55), ("instrumented", 58)):
            with self.subTest(mode=mode):
                self.prepare(mode, file_count=749)
                result = self.preflight(f"full-{mode}")
                self.assertEqual(result["status"], "passed", result["issues"])
                self.assertEqual((result["attempts"], result["scheduled"]), (count, count))
                self.assertEqual(len(self.launched), count)
                self.assertFalse(result["performance_qualified"])
                bundle_path = self.root / f"full-{mode}" / "bundle.json"
                self.assertLessEqual(bundle_path.stat().st_size, 64 * 1024 * 1024)
                bundle = collector.load(bundle_path)
                base = bundle["plan"]["binary"]
                self.assertNotIn("source_status_porcelain", base["build"])
                self.assertEqual(len(base["build"]["source_binding"]["before"]["files"]), 749)
                canonical = (json.dumps(base, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")
                reference = {"plan_binary_sha256": hashlib.sha256(canonical).hexdigest()}
                for attempt in bundle["attempts"]:
                    self.assertEqual(attempt["identity_before"], reference)
                    self.assertEqual(attempt["identity_after"], reference)
                augmented = collector.binary_identity(self.run.main, result["metadata"], TARGET)
                self.run.main.unlink()
                self.run.lib.unlink()
                self.metadata.unlink()
                shutil.rmtree(base["build"]["source_binding"]["source_root"])
                admitted = collector.runtime_closure_admission(augmented, TARGET, mode)
                self.assertEqual(admitted["closure"], bundle["closure"])
                self.assertEqual(admitted["threads"], {"threads": 4, "framework": "fixture-runtime", "reason": None})

    def test_preflight_transport_budget_preserves_identity_and_refuses_oversize(self):
        self.prepare()
        sizing = self.preflight("sizing")
        self.assertEqual(sizing["status"], "passed", sizing["issues"])
        original = collector.load(self.root / "sizing/bundle.json")
        pretty_bytes = len((json.dumps(original, sort_keys=True, indent=2, allow_nan=False) + "\n").encode())
        compact_bytes = len((json.dumps(original, sort_keys=True, separators=(",", ":"),
                                       allow_nan=False) + "\n").encode())
        budget_unit = (pretty_bytes + compact_bytes) // 128
        self.assertLess(compact_bytes, 64 * budget_unit)
        self.assertLess(64 * budget_unit, pretty_bytes)
        with mock.patch.object(collector, "MIB", budget_unit):
            result = self.preflight("bounded")
            self.assertEqual(result["status"], "passed", result["issues"])
            self.assertEqual((result["attempts"], result["scheduled"]), (55, 55))
            self.assertLessEqual((self.root / "bounded/bundle.json").stat().st_size, 64 * budget_unit)
            augmented = collector.binary_identity(self.run.main, result["metadata"], TARGET)
            collector.runtime_closure_admission(augmented, TARGET, self.build_mode)
        with mock.patch.object(collector, "MIB", compact_bytes // 128):
            oversized = self.preflight("oversized")
        self.assertEqual(oversized["status"], "incomplete")
        self.assertIn("runtime preflight bundle exceeds 64 MiB", oversized["issues"])
        self.assertFalse((self.root / "oversized/bundle.json").exists())
        self.assertFalse((self.root / "oversized/build.json").exists())

    def test_export_admission_requires_complete_exact_and_unmixed_provenance(self):
        self.prepare()
        result = self.preflight("valid")
        self.assertEqual(result["status"], "passed", result["issues"])
        original = collector.binary_identity(self.run.main, result["metadata"], TARGET)
        for change in ("variant", "porcelain", "post-proof", "incomplete", "changed", "tree", "probe", "lock"):
            with self.subTest(change=change):
                identity = copy.deepcopy(original)
                build = identity["build"]
                binding = build["source_binding"]
                if change == "variant":
                    binding["variant"] = "unchecked-export"
                elif change == "porcelain":
                    build["source_status_porcelain"] = ""
                elif change == "post-proof":
                    del binding["after"]
                elif change == "incomplete":
                    binding["before"]["complete"] = binding["after"]["complete"] = False
                elif change == "changed":
                    binding["after"]["files"]["Cargo.toml"]["sha256"] = "f" * 64
                elif change == "tree":
                    build["source_tree"] = "f" * 40
                elif change == "probe":
                    build["provenance_commands"][3]["record"]["stdout"] = "truncated source listing"
                else:
                    build["cargo_lock_sha256"] = "f" * 64
                with self.assertRaises(ValueError):
                    collector.runtime_closure_admission(identity, TARGET, "normal")

    def test_checkout_admission_preserves_clean_source_requirement(self):
        self.prepare()
        build = copy.deepcopy(self.identity["build"])
        del build["source_binding"]
        build["source_status_porcelain"] = ""
        for index, (argv, stdout) in enumerate((
            (["/git", "rev-parse", "HEAD"], build["source_revision"] + "\n"),
            (["/git", "rev-parse", "HEAD^{tree}"], build["source_tree"] + "\n"),
            (["/git", "status", "--porcelain"], ""),
        ), 1):
            build["provenance_commands"][index] = {"argv": argv, "record": process_record(stdout)}
        self.metadata.write_bytes(collector.document(build))
        self.identity = collector.binary_identity(self.run.main, self.metadata, TARGET)
        result = self.preflight("checkout")
        self.assertEqual(result["status"], "passed", result["issues"])
        identity = collector.binary_identity(self.run.main, result["metadata"], TARGET)
        collector.runtime_closure_admission(identity, TARGET, "normal")
        identity["build"]["source_status_porcelain"] = " M Cargo.lock\n"
        identity["build"]["provenance_commands"][3]["record"]["stdout"] = " M Cargo.lock\n"
        with self.assertRaisesRegex(ValueError, "dirty source"):
            collector.runtime_closure_admission(identity, TARGET, "normal")

    def test_both_boundary_references_bind_every_canonical_identity_field(self):
        self.prepare()
        result = self.preflight("valid")
        self.assertEqual(result["status"], "passed", result["issues"])
        original = collector.load(self.root / "valid" / "bundle.json")
        changes = [(boundary, change) for boundary in ("identity_before", "identity_after")
                   for change in ("digest", "missing", "null", "extra", "full", "build-only", "wrong-key")]
        changes += [("plan", "metadata_path"), ("plan", "metadata_sha256"), ("plan", "extra")]
        for index, (boundary, change) in enumerate(changes):
            with self.subTest(boundary=boundary, change=change):
                bundle = copy.deepcopy(original)
                base = bundle["plan"]["binary"]
                attempt = bundle["attempts"][-1]
                if boundary == "plan":
                    if change == "extra":
                        base["unobserved_identity_field"] = True
                    else:
                        base[change] = "/other/metadata.json" if change == "metadata_path" else "f" * 64
                elif change == "digest":
                    attempt[boundary]["plan_binary_sha256"] = "f" * 64
                elif change == "missing":
                    del attempt[boundary]
                elif change == "null":
                    attempt[boundary] = None
                elif change == "extra":
                    attempt[boundary]["unobserved_identity_field"] = True
                elif change == "full":
                    attempt[boundary] = copy.deepcopy(base)
                elif change == "build-only":
                    attempt[boundary] = {"plan_binary_sha256": hashlib.sha256(
                        collector.document(base["build"])).hexdigest()}
                else:
                    attempt[boundary] = {"sha256": attempt[boundary]["plan_binary_sha256"]}
                identity = self.retained_identity(bundle, f"tampered-{index}")
                with self.assertRaisesRegex(ValueError, "runtime preflight file identity"):
                    collector.runtime_closure_admission(identity, TARGET, "normal")

    def test_actual_before_and_after_file_changes_keep_unobserved_boundary_null(self):
        self.prepare()
        for stage, kind, message in (
            ("before", "binary", "binary changed"), ("after", "binary", "binary changed"),
            ("after", "metadata", "source metadata changed"), ("after", "dependency", "native dependency changed"),
        ):
            with self.subTest(stage=stage, kind=kind):
                victim = {"binary": self.run.main, "metadata": self.metadata, "dependency": self.run.lib}[kind]
                original = victim.read_bytes()
                save = collector.save
                self.launched.clear()

                def save_started(path, value, **options):
                    save(path, value, **options)
                    if Path(path).parent.name == "started" and stage == "before":
                        victim.write_bytes(original + b"changed at the boundary")

                def discover(argv, **options):
                    receipt = self.discover(argv, **options)
                    victim.write_bytes(original + b"changed at the boundary")
                    return receipt

                name = f"changed-{stage}-{kind}"
                try:
                    with mock.patch.object(collector, "save", side_effect=save_started):
                        result = self.preflight(name, discover)
                finally:
                    victim.write_bytes(original)
                self.assertEqual(result["status"], "incomplete")
                attempt = collector.load(self.root / name / "attempts/0000.json")
                self.assertEqual(attempt["identity_error"], message)
                self.assertIsNone(attempt["identity_after"])
                if stage == "before":
                    self.assertIsNone(attempt["identity_before"])
                    self.assertIsNone(attempt["receipt"])
                    self.assertEqual(self.launched, [])
                else:
                    self.assertEqual(attempt["identity_before"], {
                        "plan_binary_sha256": hashlib.sha256(collector.document(self.identity)).hexdigest()})
                    self.assertTrue(collector.successful(attempt["receipt"]["record"]))
                    self.assertEqual(len(self.launched), 1)
                self.assertFalse((self.root / name / "bundle.json").exists())
                started = collector.load(self.root / name / "started/0000.json")
                self.assertIsNone(started["identity_before"])
                self.assertIsNone(started["identity_after"])

    def test_interruption_and_failed_observer_keep_observed_boundary_proofs(self):
        self.prepare()
        reference = {"plan_binary_sha256": hashlib.sha256(collector.document(self.identity)).hexdigest()}

        def interrupted(_argv, **_options):
            raise KeyboardInterrupt("observer interrupted")

        with self.assertRaisesRegex(KeyboardInterrupt, "observer interrupted"):
            self.preflight("interrupted", interrupted)
        attempt = collector.load(self.root / "interrupted/attempts/0000.json")
        self.assertEqual(attempt["identity_before"], reference)
        self.assertIsNone(attempt["identity_after"])
        self.assertIsNone(attempt["identity_error"])
        self.assertIsNone(attempt["receipt"])
        self.assertEqual(attempt["interruption"], "KeyboardInterrupt")
        self.assertFalse((self.root / "interrupted/report.json").exists())

        def failed(argv, **options):
            receipt = self.discover(argv, **options)
            receipt["record"]["exit_code"] = 23
            return receipt

        result = self.preflight("failed", failed)
        self.assertEqual(result["status"], "incomplete")
        attempt = collector.load(self.root / "failed/attempts/0000.json")
        self.assertEqual(attempt["identity_before"], reference)
        self.assertEqual(attempt["identity_after"], reference)
        self.assertEqual(attempt["receipt"]["record"]["exit_code"], 23)
        self.assertEqual(collector.strict_json(attempt["receipt"]["record"]["stdout"]),
                         {**self.declaration, "timing_boundaries_enabled": True})
        self.assertFalse((self.root / "failed/bundle.json").exists())


def v4_fields():
    return {"format_version": 4, "sampling_protocol": "mado-component-sampling-v1",
            "sampling_limits": {"warmups_min": 1, "warmups_max": 1000, "samples_min": 1, "samples_max": 40000},
            "retained_loop_cpu_protocol": "mado-component-retained-loop-cpu-v1"}


def v4_manifest(build_mode="normal", role="baseline"):
    return {**v3_manifest(build_mode, role), **v4_fields()}


def v4_payload(workload="single-small96x64", warmups=2, count=7):
    value = {**v3_payload(workload, smoke=True), **v4_fields(), "warmups": warmups, "sample_count": count}
    row = value["workloads"][0]
    sample = copy.deepcopy(row["samples"][0])
    row.update(warmups=warmups, sample_count=count,
               warmup_samples=[copy.deepcopy(sample) for _ in range(warmups)],
               samples=[copy.deepcopy(sample) for _ in range(count)])
    row["metrics"].update(iterations=warmups + count, cpu_ns=sample["cpu_ns"] * count)
    if workload in collector.SESSION_DEFINITIONS:
        row["unit_observations"] = [v3_unit(row, index, "normal") for index in range(warmups + count)]
    row["retained_loop_cpu"] = {
        "protocol": "mado-component-retained-loop-cpu-v1", "source": "getrusage-RUSAGE_SELF",
        "source_unit_ns": 1000, "start_ns": 10_000_000,
        "end_ns": 10_000_000 + row["metrics"]["cpu_ns"] + 100_000,
        "cpu_ns": row["metrics"]["cpu_ns"] + 100_000, "sample_count": count, "reason": None,
        "scope": "current-process-retained-loop-including-preparation-oracles-and-storage"}
    return value


def v4_identity(root, build_mode="normal", role="baseline"):
    binding = v3_identity(root, build_mode, role)
    declaration = v4_manifest(build_mode, role)
    build = binding["build"]
    build.update(protocol_version=4,
                 benchmark_manifest=collector.manifest_identity(declaration, TARGET, "4.12.0", 4, build_mode))
    build["provenance_commands"][0] = {
        "argv": [binding["path"], "--protocol-version", "4", "--list"], "record": process_record(declaration)}
    return binding


def v4_parsed_evidence(root, mode, build_mode="normal", workload="selection-k16-dense1080p-keepall"):
    # Synthetic scalar cohorts exercise the frozen arithmetic, not native eligibility.
    evidence = v3_parsed_evidence(root, mode, build_mode, workload)
    plan = evidence["plan"]
    plan.update(format_version=4, policy=copy.deepcopy(collector.V4_POLICY))
    plan["binaries"] = {role: v4_identity(root, build_mode, "baseline" if role == "control" else role)
                        for role in plan["binaries"]}
    evidence["manifest"] = plan["binaries"]["baseline"]["build"]["benchmark_manifest"]
    counts = {"warmups": 20, "samples": 200} if collector.V3_DEFINITIONS[workload]["statistical"] else {
        "warmups": 1, "samples": 3}
    sampling = {workload: counts}
    raw = json.dumps(sampling)
    plan["sampling"] = {"path": str(root / "sampling.json"), "raw": raw, "rows": sampling,
                        "sha256": hashlib.sha256(raw.encode()).hexdigest()}
    for workloads in evidence["observations"].values():
        for row in workloads[workload]:
            row["retained_loop_cpu_ns"] = row["cpu_ns"] + 100_000
    return evidence


class ProtocolFourTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.workload = "single-small96x64"
        self.manifest = collector.manifest_identity(v4_manifest(), TARGET, "4.12.0", 4, "normal")

    def binding(self, rows):
        raw = json.dumps(rows)
        return {"path": str(self.root / "sampling.json"), "raw": raw, "rows": copy.deepcopy(rows),
                "sha256": hashlib.sha256(raw.encode()).hexdigest()}

    def parse(self, value, workload=None, counts=None):
        return collector.observations(value, self.manifest, workload or self.workload, False,
                                      sampling=counts or {"warmups": 2, "samples": 7})

    def test_actual_custom_counts_and_contained_cpu_are_independently_checked(self):
        for workload in (self.workload, "session-retained-rgba-roi-hit"):
            value = v4_payload(workload)
            observed = self.parse(value, workload)
            self.assertEqual(observed["cpu_ns"], sum(item["cpu_ns"] for item in value["workloads"][0]["samples"]))
            self.assertEqual(observed["retained_loop_cpu_ns"], observed["cpu_ns"] + 100_000)
            with self.assertRaises(ValueError):
                self.parse(value, workload, {"warmups": 2, "samples": 6})
            value["sample_count"] = 200
            with self.assertRaises(ValueError):
                self.parse(value, workload)

    def test_loop_cpu_rejects_invalid_arithmetic_sources_and_hidden_availability(self):
        variants = [
            {"start_ns": True}, {"end_ns": 1 << 64}, {"cpu_ns": 1}, {"sample_count": 6},
            {"source": "GetProcessTimes-current-process"}, {"source_unit_ns": 100},
            {"scope": "owned-process-tree"}, {"reason": "not available"},
            {"cpu_ns": None, "reason": "not available"}, {"end_ns": 0},
        ]
        for change in variants:
            with self.subTest(change=change), self.assertRaises(ValueError):
                value = v4_payload()
                value["workloads"][0]["retained_loop_cpu"].update(change)
                self.parse(value)
        value = v4_payload()
        bracket = value["workloads"][0]["retained_loop_cpu"]
        bracket.update(start_ns=None, cpu_ns=None, reason="native counter unavailable")
        summary = self.parse(value)
        self.assertIsNone(summary["retained_loop_cpu_ns"])
        self.assertIn("retained_loop_cpu_ns", summary["unavailable"])

    def test_missing_capability_or_bracket_refuses_new_protocol(self):
        for key in ("sampling_protocol", "sampling_limits", "retained_loop_cpu_protocol"):
            value = v4_payload()
            del value[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.parse(value)
        value = v4_payload()
        del value["workloads"][0]["retained_loop_cpu"]
        with self.assertRaises(ValueError):
            self.parse(value)

    def test_sampling_requires_exact_hash_coverage_and_bounded_integer_counts(self):
        definitions = {self.workload: collector.V3_DEFINITIONS[self.workload]}
        rows = {self.workload: {"warmups": 20, "samples": 2000}}
        binding = self.binding(rows)
        self.assertEqual(collector.validate_sampling(binding, definitions), rows)
        changes = (
            ("warmups", 0), ("warmups", 1001), ("samples", 0), ("samples", 40001),
            ("samples", True), ("samples", 3.5),
        )
        for field, number in changes:
            changed = copy.deepcopy(rows)
            changed[self.workload][field] = number
            with self.subTest(field=field, number=number), self.assertRaises(ValueError):
                collector.validate_sampling(self.binding(changed), definitions)
        for changed in ({}, {**rows, "unknown": {"warmups": 1, "samples": 3}}):
            with self.assertRaises(ValueError):
                collector.validate_sampling(self.binding(changed), definitions)
        binding["rows"][self.workload]["samples"] = 200
        with self.assertRaises(ValueError):
            collector.validate_sampling(binding, definitions)
        binding = self.binding(rows)
        binding["sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            collector.validate_sampling(binding, definitions)

    def test_contract_rows_cannot_adopt_statistical_counts(self):
        name = "query-deadline-rgba-roi"
        definitions = {name: collector.V3_DEFINITIONS[name]}
        with self.assertRaises(ValueError):
            collector.validate_sampling(self.binding({name: {"warmups": 20, "samples": 200}}), definitions)
        rows = {name: {"warmups": 1, "samples": 3}}
        self.assertEqual(collector.validate_sampling(self.binding(rows), definitions), rows)
        value = v4_payload(name, 1, 3)
        self.assertEqual(self.parse(value, name, rows[name])["metrics"]["iterations"], 4)
        value.update(warmups=20, sample_count=200)
        with self.assertRaises(ValueError):
            self.parse(value, name, rows[name])

    def test_changed_frozen_counts_refuse_comparison_before_collection(self):
        sampling = self.binding({self.workload: {"warmups": 20, "samples": 200}})
        plan = {"format_version": 4, "build_mode": "normal", "mode": "comparison",
                "workloads": [self.workload], "sampling": sampling}
        profile = {"workloads": plan["workloads"], "envelopes": {self.workload: {}},
                   "sampling": copy.deepcopy(sampling)}
        collector.comparison_scope(plan, profile)
        profile["sampling"] = self.binding({self.workload: {"warmups": 20, "samples": 201}})
        with self.assertRaises(ValueError):
            collector.comparison_scope(plan, profile)

    def test_normal_loop_cpu_and_instrumented_resource_gates_stay_separate(self):
        for mode in ("normal", "instrumented"):
            evidence = v4_parsed_evidence(self.root, "calibration", mode, self.workload)
            for row in evidence["observations"]["baseline"][self.workload]:
                row["retained_loop_cpu_ns"] = 200_000_000
                row["cpu_ns"] = row["process"] * 100_000_000
            envelope = collector.derive_envelopes(evidence)[self.workload]
            self.assertNotIn("cpu_ns", envelope["benefit_allowances"])
            self.assertEqual(set(envelope["empirical_allowances"]), set(envelope["benefit_allowances"]))
            self.assertEqual(set(envelope["control_allowances"]), set(envelope["benefit_allowances"]))
            self.assertIn("peak_rss_bytes", envelope["limits"])
            if mode == "normal":
                self.assertEqual(set(envelope["benefit_allowances"]), {"p95_ns", "retained_loop_cpu_ns"})
                self.assertEqual(envelope["empirical_allowances"]["retained_loop_cpu_ns"], 2000)
                self.assertEqual(envelope["control_allowances"]["retained_loop_cpu_ns"], 30_000_000)
                self.assertEqual(envelope["limits"]["retained_loop_cpu_ns"], 200_002_000)
                evidence["observations"]["baseline"][self.workload][0]["retained_loop_cpu_ns"] = 400_000_000
                with self.assertRaises(ValueError):
                    collector.derive_envelopes(evidence)
            else:
                self.assertEqual(envelope["benefit_allowances"], {})
                self.assertIn("peak_heap_bytes", envelope["limits"])


class ProtocolFourControlBudgetTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.workload = "selection-k16-dense1080p-keepall"
        self.calibration, self.control, self.comparison = (
            v4_parsed_evidence(self.root, mode, workload=self.workload)
            for mode in ("calibration", "control", "comparison"))
        self.profile = v2_numeric_profile(self.calibration)

    def freeze(self):
        binding = save_method_review(self.root, self.comparison["plan"])
        for evidence in (self.calibration, self.control, self.comparison):
            plan = evidence["plan"]
            plan["method_review"] = copy.deepcopy(binding)
            plan["execution_limits"] = copy.deepcopy(binding["stage_limits"][plan["mode"]])
        plan = self.calibration["plan"]
        review = {"format_version": 4, "kind": "component-profile-review", "decision": "accepted",
                  "target": TARGET, "build_mode": "normal", "workloads": plan["workloads"],
                  "sampling": plan["sampling"], "method_review": binding,
                  "calibration": collector.cohort_binding(self.calibration),
                  "control": collector.cohort_binding(self.control),
                  "envelopes": collector.derive_envelopes(self.calibration),
                  "reviewer": "fixture reviewer", "rationale": "synthetic control-budget arithmetic only"}
        path = self.root / "profile-review.json"
        collector.save(path, review)
        with mock.patch.object(collector, "runtime_closure_admission"):
            return collector.profile_from_v2(
                self.calibration, self.root / "calibration", self.control, self.root / "control",
                "fixture reviewer", "synthetic control-budget arithmetic only", path, collector.sha256(path))

    def test_own_metric_half_integer_medians_are_rounded_up_exactly(self):
        cases = (
            ("p95_ns", 100_000, 20, 15_001),
            ("retained_loop_cpu_ns", 900_000, 2000, 135_001),
            ("p95_ns", 10**18 + 6, 20, 150_000_000_000_000_001),
        )
        for metric, low, empirical, control in cases:
            calibration = copy.deepcopy(self.calibration)
            rows = calibration["observations"]["baseline"][self.workload]
            for row, value in zip(rows, (low, low + 1, low, low + 1, low, low + 1)):
                row[metric] = value
            envelope = collector.derive_envelopes(calibration)[self.workload]
            with self.subTest(metric=metric, low=low):
                self.assertEqual(envelope["empirical_allowances"][metric], empirical)
                self.assertEqual(envelope["control_allowances"][metric], control)
                self.assertEqual(envelope["benefit_allowances"][metric], control)
                self.assertIs(type(envelope["control_allowances"][metric]), int)
                self.assertEqual(envelope["limits"][metric], low + 1 + empirical)
                other = "retained_loop_cpu_ns" if metric == "p95_ns" else "p95_ns"
                for field in ("empirical_allowances", "control_allowances", "benefit_allowances", "limits"):
                    self.assertEqual(envelope[field][other], self.profile["envelopes"][self.workload][field][other])

    def test_control_budget_does_not_enlarge_individual_timing_or_resource_limits(self):
        historical = copy.deepcopy(self.calibration)
        historical["plan"].update(format_version=3, policy=copy.deepcopy(collector.V3_POLICY))
        for row in historical["observations"]["baseline"][self.workload]:
            row["cpu_ns"] = row["retained_loop_cpu_ns"]
        old = collector.derive_envelopes(historical)[self.workload]
        envelope = self.profile["envelopes"][self.workload]
        for field, values in (("limits", old["limits"]), ("empirical_allowances", old["benefit_allowances"]),
                              ("clock_allowances", old["clock_allowances"])):
            expected = {("retained_loop_cpu_ns" if key == "cpu_ns" else key): value
                        for key, value in values.items()}
            self.assertEqual(envelope[field], expected)
        for metric in ("p95_ns", "retained_loop_cpu_ns"):
            self.assertGreater(envelope["control_allowances"][metric], envelope["empirical_allowances"][metric])
        for mode, original in (("control", self.control), ("comparison", self.comparison)):
            for role in original["observations"]:
                for metric in envelope["limits"]:
                    changed = copy.deepcopy(original)
                    changed["observations"][role][self.workload][-1][metric] = envelope["limits"][metric] + 1
                    issues = (collector.control_evidence(self.calibration, changed) if mode == "control"
                              else collector.compare_evidence(changed, self.profile)[0])
                    with self.subTest(mode=mode, role=role, metric=metric):
                        self.assertTrue(any(issue["attempt"] == f"{role}:{self.workload}:5"
                                            and f"{metric}=" in issue["reason"] for issue in issues))

    def test_target_cpu_floor_and_zero_clock_aware_guards_are_unchanged(self):
        for target, floor in ((TARGET, 1000), (collector.TARGETS[1], 15_625_000)):
            calibration = copy.deepcopy(self.calibration)
            calibration["plan"]["target"] = target
            for row in calibration["observations"]["baseline"][self.workload]:
                row.update(p95_ns=0, retained_loop_cpu_ns=0)
            envelope = collector.derive_envelopes(calibration)[self.workload]
            with self.subTest(target=target):
                self.assertEqual(envelope["clock_allowances"]["retained_loop_cpu_ns"], floor)
                for field in ("limits", "empirical_allowances", "control_allowances", "benefit_allowances"):
                    self.assertEqual(envelope[field]["p95_ns"], 20)
                    self.assertEqual(envelope[field]["retained_loop_cpu_ns"], 2 * floor)

    def test_control_bias_uses_own_frozen_budget_in_both_directions(self):
        envelope = self.profile["envelopes"][self.workload]
        for metric, allowance in envelope["control_allowances"].items():
            reference = self.calibration["observations"]["baseline"][self.workload][0][metric]
            for role in ("baseline", "control"):
                for excess in (0, 1):
                    changed = copy.deepcopy(self.control)
                    for row in changed["observations"][role][self.workload]:
                        row[metric] = reference - allowance - excess
                    issues = collector.control_evidence(self.calibration, changed)
                    with self.subTest(metric=metric, role=role, excess=excess):
                        self.assertEqual(bool(issues), bool(excess))
                        self.assertTrue(all(f"{metric} between-role bias" in issue["reason"] for issue in issues))

    def test_control_and_comparison_order_drift_keep_separate_noise_gate(self):
        envelope = self.profile["envelopes"][self.workload]
        for mode, original in (("control", self.control), ("comparison", self.comparison)):
            for role in original["observations"]:
                for metric, control in envelope["control_allowances"].items():
                    for kind in ("order", "drift"):
                        for delta in (envelope["empirical_allowances"][metric] + 1, control, control + 1):
                            changed = copy.deepcopy(original)
                            rows = changed["observations"][role][self.workload]
                            # Both roles use the calibration scale here; benefit is an independent gate.
                            reference = self.calibration["observations"]["baseline"][self.workload][0][metric]
                            for row in rows:
                                shifted = row["position"] == 0 if kind == "order" else row["process"] >= 3
                                row[metric] = reference - (delta if shifted else 0)
                            issues = (collector.control_evidence(self.calibration, changed) if mode == "control"
                                      else collector.compare_evidence(changed, self.profile)[0])
                            with self.subTest(mode=mode, role=role, metric=metric, kind=kind, delta=delta):
                                self.assertEqual(any(f"{metric} {kind}" in issue["reason"]
                                                     and issue["attempt"] == f"{role}:{self.workload}"
                                                     for issue in issues), delta > control)
                                self.assertEqual(any(f"{metric} noise" in issue["reason"]
                                                     and issue["attempt"] == f"{role}:{self.workload}"
                                                     for issue in issues), delta >= control)

    def test_comparison_role_noise_keeps_exact_declared_boundary(self):
        for role in ("baseline", "candidate"):
            for excess in (0, 1):
                changed = copy.deepcopy(self.comparison)
                for row in changed["observations"][role][self.workload]:
                    row["p95_ns"] = 800_000 if row["process"] < 4 else 920_000 + excess
                issues, _ = collector.compare_evidence(changed, self.profile)
                with self.subTest(role=role, excess=excess):
                    self.assertEqual(any("p95_ns noise" in issue["reason"]
                                         and issue["attempt"] == f"{role}:{self.workload}" for issue in issues),
                                     bool(excess))

    def test_joint_benefits_must_strictly_exceed_control_not_just_empirical_error(self):
        envelope = self.profile["envelopes"][self.workload]
        self.assertEqual(collector.compare_evidence(self.comparison, self.profile)[0], [])
        for metric, control in envelope["control_allowances"].items():
            for delta in (envelope["empirical_allowances"][metric] + 1, control, control + 1):
                changed = copy.deepcopy(self.comparison)
                reference = changed["observations"]["baseline"][self.workload][0][metric]
                for row in changed["observations"]["candidate"][self.workload]:
                    row[metric] = reference - delta
                issues, benefits = collector.compare_evidence(changed, self.profile)
                with self.subTest(metric=metric, delta=delta):
                    self.assertEqual(benefits[self.workload][metric]["allowance"], control)
                    self.assertEqual(benefits[self.workload][metric]["passed"], delta > control)
                    self.assertEqual(bool(issues), delta <= control)
                    other = "retained_loop_cpu_ns" if metric == "p95_ns" else "p95_ns"
                    self.assertTrue(benefits[self.workload][other]["passed"])
        calibration = v4_parsed_evidence(self.root, "calibration", workload="single-small96x64")
        comparison = v4_parsed_evidence(self.root, "comparison", workload="single-small96x64")
        self.assertEqual(collector.compare_evidence(comparison, v2_numeric_profile(calibration)), ([], {}))

    def test_recomputed_profile_rejects_every_allowance_and_bound_perturbation(self):
        profile = self.freeze()
        with (mock.patch.object(collector, "evaluate", side_effect=[self.calibration, self.control]),
              mock.patch.object(collector, "runtime_closure_admission")):
            self.assertEqual(collector.validate_profile(profile), profile)
        fields = ("empirical_allowances", "control_allowances", "benefit_allowances", "clock_allowances", "limits")
        for field in fields:
            for mutation in ("increase", "decrease", "float", "remove"):
                changed = copy.deepcopy(profile)
                values = changed["envelopes"][self.workload][field]
                if mutation == "remove":
                    del changed["envelopes"][self.workload][field]
                else:
                    value = values["p95_ns"]
                    values["p95_ns"] = float(value) if mutation == "float" else value + (
                        1 if mutation == "increase" else -1)
                path = self.root / f"profile-{field}-{mutation}.json"
                collector.save(path, changed)
                with (self.subTest(field=field, mutation=mutation),
                      mock.patch.object(collector, "evaluate", side_effect=[self.calibration, self.control]),
                      mock.patch.object(collector, "runtime_closure_admission"),
                      self.assertRaisesRegex(ValueError, "profile/provenance/envelopes")):
                    collector.validate_profile(collector.load(path))

    def test_rehashed_profile_review_cannot_authorize_edited_allowances(self):
        profile = self.freeze()
        original = collector.load(profile["profile_review"]["path"])
        for field in ("empirical_allowances", "control_allowances", "benefit_allowances", "limits"):
            review = copy.deepcopy(original)
            review["envelopes"][self.workload][field]["retained_loop_cpu_ns"] += 1
            path = self.root / f"review-{field}.json"
            collector.save(path, review)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "exact method/cohorts/envelopes"):
                collector.profile_from_v2(
                    self.calibration, profile["calibration"]["path"], self.control, profile["control"]["path"],
                    "fixture reviewer", "synthetic control-budget arithmetic only", path, collector.sha256(path))

    def test_old_policy_method_profile_and_source_references_are_not_admitted(self):
        profile = self.freeze()
        collector.validate_plan(self.calibration["plan"])
        old_policy = {**copy.deepcopy(collector.V3_POLICY),
                      "id": "template-efficiency-v4-retained-loop-prospective",
                      **{key: collector.V4_POLICY[key] for key in
                         ("cpu_metric", "sampling", "instrumented_timing", "scope")}}
        changed = copy.deepcopy(self.calibration)
        changed["plan"]["policy"] = old_policy
        for operation in (lambda: collector.validate_plan(changed["plan"]),
                          lambda: collector.derive_envelopes(changed)):
            with self.assertRaisesRegex(ValueError, "policy differs"):
                operation()
        old_profile = copy.deepcopy(profile)
        old_profile["policy"] = old_policy
        with self.assertRaisesRegex(ValueError, "policy differs"):
            collector.validate_profile(old_profile)
        review = collector.load(profile["method_review"]["path"])
        review["policy"] = old_policy
        path = self.root / "old-policy-method.json"
        collector.save(path, review)
        with self.assertRaisesRegex(ValueError, "method review policy mismatch"):
            collector.reviewed_method(path, collector.sha256(path), self.calibration["plan"])
        changed = copy.deepcopy(self.calibration["plan"])
        changed["collector"]["tools/template-matching-efficiency.py"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "collector source differs"):
            collector.validate_plan(changed)
        changed = copy.deepcopy(self.control)
        changed["plan"]["method_review"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "cohort method_review"):
            collector.control_evidence(self.calibration, changed)
        changed = copy.deepcopy(self.comparison)
        changed["plan"]["policy"] = old_policy
        with self.assertRaisesRegex(ValueError, "policy differs"):
            collector.compare_evidence(changed, old_profile)

    def test_historical_protocols_keep_empirical_benefits_and_control_semantics(self):
        for version, fixture in ((1, parsed_evidence), (2, v2_parsed_evidence), (3, v3_parsed_evidence)):
            calibration = fixture(self.root, "calibration")
            comparison = fixture(self.root, "comparison")
            envelope = collector.derive_envelopes(calibration)[self.workload]
            with self.subTest(version=version):
                self.assertNotIn("empirical_allowances", envelope)
                self.assertNotIn("control_allowances", envelope)
                self.assertEqual(envelope["benefit_allowances"]["p95_ns"], 20)
                self.assertEqual(envelope["benefit_allowances"]["cpu_ns"], 2000)
                for row in comparison["observations"]["candidate"][self.workload]:
                    row["p95_ns"] = 1_000_000 - 21
                profile = (collector.profile_from(calibration, self.root, "fixture", "historical arithmetic")
                           if version == 1 else v2_numeric_profile(calibration))
                self.assertEqual(collector.compare_evidence(comparison, profile)[0], [])
                if version >= 2:
                    control = fixture(self.root, "control")
                    for row in control["observations"]["control"][self.workload]:
                        row["p95_ns"] -= 21
                    self.assertTrue(any("p95_ns between-role bias" in issue["reason"]
                                        for issue in collector.control_evidence(calibration, control)))

    def test_public_policy_cli_exposes_only_new_protocol4_policy(self):
        identifiers = {
            1: "template-efficiency-v1", 2: "template-efficiency-v2-prospective-r2",
            3: "template-efficiency-v3-session-prospective-r2",
            4: "template-efficiency-v4-retained-loop-control-budget-prospective-r2",
        }
        for version, identifier in identifiers.items():
            with self.subTest(version=version), mock.patch("builtins.print") as output:
                self.assertEqual(collector.main(["policy", "--protocol-version", str(version),
                                                 "--build-mode", "normal"]), 0)
            policy = json.loads(output.call_args.args[0])["policy"]
            self.assertEqual(policy["id"], identifier)
            if version == 4:
                self.assertEqual(policy["control_relative_allocation"], {"numerator": 15, "denominator": 100})
            else:
                self.assertNotIn("control_relative_allocation", policy)

    def test_large_half_ns_medians_cannot_round_control_or_benefit_across_threshold(self):
        reference = 10**18 + 6
        for metric in ("p95_ns", "retained_loop_cpu_ns"):
            calibration = copy.deepcopy(self.calibration)
            for row in calibration["observations"]["baseline"][self.workload]:
                row[metric] = reference + row["process"] % 2
            profile = v2_numeric_profile(calibration)
            allowance = profile["envelopes"][self.workload]["control_allowances"][metric]
            self.assertEqual(allowance, 150_000_000_000_000_001)
            for excess in (0, 1):
                control = copy.deepcopy(self.control)
                for role, rows in control["observations"].items():
                    for row in rows[self.workload]:
                        row[metric] = reference + row["process"] % 2 - (
                            allowance + excess if role == "control" else 0)
                with self.subTest(metric=metric, excess=excess):
                    issues = collector.control_evidence(calibration, control)
                    self.assertEqual(bool(issues), bool(excess))
            for shortfall in (0, 1):
                comparison = copy.deepcopy(self.comparison)
                for row in comparison["observations"]["baseline"][self.workload]:
                    row[metric] = reference + row["process"] % 2
                for row in comparison["observations"]["candidate"][self.workload]:
                    row[metric] = reference - allowance + shortfall
                issues, benefits = collector.compare_evidence(comparison, profile)
                result = benefits[self.workload][metric]
                with self.subTest(metric=metric, shortfall=shortfall):
                    self.assertEqual(result["passed"], shortfall == 0)
                    self.assertEqual(bool(issues), shortfall == 1)
                    self.assertEqual(result["baseline_median_twice_ns"], 2 * reference + 1)
                    self.assertEqual(result["candidate_median_twice_ns"], 2 * (reference - allowance + shortfall))
                    self.assertEqual(json.loads(collector.document(result)), result)

    def test_turnover_strata_retain_independent_wall_budgets_without_stratum_cpu(self):
        workload = "session-turnover-rgba-full"
        calibration = v4_parsed_evidence(self.root, "calibration", workload=workload)
        for row in calibration["observations"]["baseline"][workload]:
            row["strata.hit.p95_ns"] = 2_000_000
            row["strata.miss.p95_ns"] = 4_000_000
        envelope = collector.derive_envelopes(calibration)[workload]
        for name, median in (("hit", 2_000_000), ("miss", 4_000_000)):
            metric = f"strata.{name}.p95_ns"
            self.assertEqual(envelope["empirical_allowances"][metric], 20)
            self.assertEqual(envelope["control_allowances"][metric], median * 15 // 100)
            self.assertEqual(envelope["benefit_allowances"][metric], median * 15 // 100)
            self.assertEqual(envelope["limits"][metric], median + 20)
            self.assertNotIn(f"strata.{name}.cpu_ns", envelope["limits"])


if __name__ == "__main__":
    unittest.main()
