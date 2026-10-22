"""Exercise provenance, immutable profiles, failed rows, and numerical acceptance without Cargo."""

import contextlib
import copy
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tempfile
import unittest
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
                    "smoke", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "build.json",
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
            "smoke", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "build.json",
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
            "attest", "--target", TARGET, "--binary", str(binary), "--build-argv-json", '["cargo","bench"]',
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
            "attest", "--target", TARGET, "--binary", str(binary), "--build-argv-json", '["cargo","bench"]',
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
            "smoke", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "build.json",
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

    def test_candidate_cannot_write_inside_calibration_or_start_with_an_unpinned_profile(self):
        calibration = self.root / "calibration"
        persist_campaign(calibration, "calibration")
        profile = collector.profile_from(collector.evaluate(calibration), calibration, "reviewer", "fixed host")
        profile_path = self.root / "profile.json"
        collector.save(profile_path, profile)
        destination = calibration / "candidate-results"
        args = collector.parser().parse_args([
            "compare", "--target", TARGET, "--baseline", "baseline", "--baseline-metadata", "base.json",
            "--candidate", "candidate", "--candidate-metadata", "candidate.json", "--exclusive-host",
            "--profile", str(profile_path), "--profile-sha256", collector.sha256(profile_path),
            "--output", str(destination)])
        with (mock.patch.object(collector, "host_identity", return_value=profile["host"]),
              mock.patch.object(collector, "environment", return_value=profile["environment"]),
              mock.patch.object(collector, "binary_identity",
                                side_effect=lambda binary, metadata, target: identity(self.root, binary)),
              mock.patch.object(collector, "run_process") as launch):
            with self.assertRaisesRegex(ValueError, "must not write into calibration"):
                collector.collect(args, "comparison")
            args.profile_sha256 = "0" * 64
            with self.assertRaisesRegex(ValueError, "reviewed profile hash mismatch"):
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
                "smoke", "--target", TARGET, "--baseline", "binary", "--baseline-metadata", "build.json",
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
    return {"format_version": 2, **{key: specification[key] for key in
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

    def launch_smoke(self, args, records, clock=None):
        original_runtime = collector.loaded_runtime
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(collector, "host_identity", return_value={"target": TARGET}))
            stack.enter_context(mock.patch.object(collector, "binary_identity",
                                                  return_value=v2_identity(self.root, args.build_mode)))
            stack.enter_context(mock.patch.object(collector, "check_files"))
            stack.enter_context(mock.patch.object(collector, "exclusive_collector",
                                                  return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(
                collector, "loaded_runtime",
                side_effect=lambda value, identity=None, **kwargs: original_runtime(value, identity)))
            if clock is not None:
                stack.enter_context(mock.patch.object(collector.time, "monotonic", side_effect=clock))
            launch = stack.enter_context(mock.patch.object(collector, "run_process", side_effect=records))
            result = collector.collect(args, "smoke")
        return result, launch

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


def save_method_review(root, specification):
    review = {"format_version": 2, "kind": "component-method-review", "decision": "accepted",
              **{key: specification[key] for key in
                 ("target", "build_mode", "host", "environment", "collector", "policy", "workloads")},
              "baseline": specification["binaries"]["baseline"],
              "candidate": specification["binaries"].get("candidate")
                           or v2_identity(root, specification["build_mode"], "candidate"),
              "reviewer": "fixture reviewer", "rationale": "deterministic apparatus decision only",
              "source_host_cost_review": "synthetic finite component plan; no runtime clearance",
              "runtime_observer_eligibility_review": "synthetic receipt validation only; no native eligibility claim",
              "stage_limits": {mode: collector.execution_limits(
                  mode, len(collector.schedule(mode, specification["workloads"], 2)))
                  for mode in ("calibration", "control", "comparison")}}
    path = root / "method.json"
    collector.save(path, review)
    return {"path": str(path), "sha256": collector.sha256(path)}


class ProtocolTwoAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.calibration = v2_parsed_evidence(self.root, "calibration")
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
              mock.patch.object(collector, "binary_identity", return_value=v2_identity(self.root)),
              mock.patch.object(collector, "run_process") as launch,
              self.assertRaisesRegex(ValueError, "missing runtime closure.*zero launches")):
            collector.collect(args, "calibration")
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
                      mock.patch.object(collector, "binary_identity", return_value=v2_identity(self.root)),
                      mock.patch.object(collector, "run_process") as launch,
                      self.assertRaisesRegex(ValueError, "method review")):
                    collector.collect(args, "calibration")
                launch.assert_not_called()
                Path(self.binding["path"]).write_bytes(original)

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
              mock.patch.object(collector, "binary_identity", return_value=v2_identity(self.root)),
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
              mock.patch.object(collector, "binary_identity", return_value=v2_identity(self.root)),
              mock.patch.object(collector, "evaluate", return_value=self.calibration),
              mock.patch.object(collector, "run_process") as launch,
              self.assertRaisesRegex(ValueError, "missing runtime closure")):
            collector.collect(args, "control")
        launch.assert_not_called()
        self.assertFalse(Path(args.output).exists())

    def test_even_reviewed_numerical_profile_needs_actual_runtime_coverage(self):
        control = v2_parsed_evidence(self.root, "control")
        control["plan"]["method_review"] = self.binding
        review = {"format_version": 2, "kind": "component-profile-review", "decision": "accepted",
                  "target": TARGET, "build_mode": "normal",
                  "method_review": self.binding, "calibration": collector.cohort_binding(self.calibration),
                  "control": collector.cohort_binding(control), "envelopes": collector.derive_envelopes(self.calibration),
                  "reviewer": "fixture reviewer", "rationale": "synthetic arithmetic only"}
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

    def test_observed_nonzero_exit_keeps_post_identity_and_later_rows(self):
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
            if task["workload"] is None:
                value = collector.strict_json(binding["build"]["provenance_commands"][0]["record"]["stdout"])
            else:
                value = v2_payload(task["workload"])
                value["loaded_native_runtime"] = self.runtime
            receipt = self.run.observation(options["closure"], stdout=json.dumps(value),
                                           exit_code=23 if task["id"] == "0001" else 0)
            receipt["duration_seconds"] = 0.2
            return receipt

        with (mock.patch.object(collector, "host_identity", return_value=specification["host"]),
              mock.patch.object(collector, "environment", return_value=specification["environment"]),
              mock.patch.object(collector, "binary_identity", return_value=binding),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector.macos_images, "observe", side_effect=observe)):
            result = collector.collect(args, "calibration")
        failed = collector.load(Path(args.output) / "attempts/0001.json")
        self.assertEqual(failed["identity_after"], binding)
        self.assertIsNone(failed["identity_error"])
        self.assertEqual(launched, specification["schedule"])
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["performance_qualified"])
        self.assertIn("exit", {issue.get("category") for issue in result["issues"]})
        self.assertNotIn("identity", {issue.get("category") for issue in result["issues"]})
        self.assertFalse(any(issue["status"] == "not-run" for issue in result["issues"]))

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

    def test_interruption_survives_failed_receipt_persistence(self):
        args = collector.parser().parse_args([
            "runtime-preflight", "--protocol-version", "2", "--build-mode", "normal", "--target", TARGET,
            "--binary", "fixture", "--build-metadata", "fixture.json", "--output", str(self.root / "out")])
        original_save = collector.save

        def save(path, value):
            if Path(path).parent.name == "attempts":
                raise OSError("receipt storage failed")
            original_save(path, value)

        with (mock.patch.object(collector, "host_identity", return_value={"target": TARGET}),
              mock.patch.object(collector, "binary_identity", return_value=self.identity),
              mock.patch.object(collector, "check_files"),
              mock.patch.object(collector, "exclusive_collector", return_value=contextlib.nullcontext()),
              mock.patch.object(collector.macos_images, "discover", side_effect=KeyboardInterrupt("primary")),
              mock.patch.object(collector, "save", side_effect=save),
              self.assertRaisesRegex(KeyboardInterrupt, "primary")):
            collector.runtime_preflight(args)


if __name__ == "__main__":
    unittest.main()
