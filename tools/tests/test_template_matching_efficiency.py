"""Exercise provenance, immutable profiles, failed rows, and numerical acceptance without Cargo."""

import contextlib
import copy
import importlib.util
import json
import os
from pathlib import Path
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


def manifest():
    rows = []
    for definition in collector.DEFINITIONS.values():
        rows.append({**definition, "cache_state": "fresh-frame-and-backend",
                     "timing_boundary": "all public searches; setup excluded",
                     "width": 1920, "height": 1080, "result_limit": 16,
                     "suppression": "KeepAll", "warmups": 20 if definition["statistical"] else 1,
                     "sample_count": 200 if definition["statistical"] else 3})
    return {"format_version": 1, "kind": "manifest", "target": TARGET,
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
    return {"format_version": 1, "kind": "observations", "target": TARGET,
            "opencv_version": "4.12.0", "fixture_sha256": "f" * 64,
            "warmups": 1 if smoke else 20, "sample_count": 3 if smoke else 200,
            "timer_resolution_ns": 10, "cpu_timer_resolution_ns": 1000,
            "timer_resolution_method": "minimum-observed-positive-Instant-delta",
            "cpu_resolution_method": "minimum-observed-positive-process-cpu-delta", "workloads": [row]}


def identity(root, role):
    return {"path": str(root / role), "sha256": ("a" if role == "baseline" else "b") * 64,
            "metadata_path": str(root / f"{role}.json"), "metadata_sha256": "d" * 64,
            "build": {"target": TARGET, "source_revision": ("a" if role == "baseline" else "b") * 40,
                      "source_tree": "c" * 40, "cargo_lock_sha256": "d" * 64,
                      "binary_sha256": ("a" if role == "baseline" else "b") * 64,
                      "rustc": "rustc 1.97.1", "build_argv": ["cargo", "bench", "--no-run", "--locked"],
                      "opencv_version": "4.12.0", "dependencies": [{"path": str(root / "opencv"), "sha256": "e" * 64}]}}


def plan(root, mode):
    roles = ("baseline", "candidate") if mode == "comparison" else ("baseline",)
    rows = list(collector.DEFINITIONS)
    return {"format_version": 1, "kind": "campaign", "mode": mode, "target": TARGET,
            "host": {"target": TARGET, "node": "controlled-host"}, "collector": collector.collector_identity(),
            "binaries": {role: identity(root, role) for role in roles}, "environment": {},
            "policy": copy.deepcopy(collector.POLICY), "workloads": rows,
            "schedule": collector.schedule(mode, rows), "exclusive_host": True, "cwd": str(root)}


def parsed_evidence(root, mode):
    specification = plan(root, mode)
    frozen = collector.manifest_identity(manifest(), TARGET, "4.12.0")
    cohorts = {}
    for role in specification["binaries"]:
        cohorts[role] = {
            workload: [collector.observations(payload(workload, candidate=role == "candidate"), frozen, workload, False)
                       for _ in range(5)]
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
            "identity_after": binary, "identity_error": None, "record": record})
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

    def test_missing_native_memory_is_incomplete_not_a_passing_zero(self):
        value = payload(self.workload)
        value["workloads"][0]["metrics"].update(peak_rss_bytes=None, native_resource_reason="API unavailable")
        with self.assertRaises(collector.Incomplete):
            self.parse(value)
        value = payload(self.workload, smoke=True)
        value["workloads"][0]["metrics"].update(peak_rss_bytes=None, native_resource_reason="API unavailable")
        self.assertIn("peak_rss_bytes", self.parse(value, smoke=True)["unavailable"])

    def test_missing_cpu_is_incomplete_and_not_substituted(self):
        value = payload(self.workload)
        value["workloads"][0]["samples"][3]["cpu_ns"] = None
        value["workloads"][0]["metrics"].update(cpu_ns=None, cpu_reason="CPU clock unavailable")
        with self.assertRaises(collector.Incomplete):
            self.parse(value)

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
        build["binary_sha256"] = collector.sha256(binary)
        build["dependencies"][0]["sha256"] = collector.sha256(dependency)
        collector.save(metadata, build)
        binding = collector.binary_identity(binary, metadata, TARGET)
        dependency.write_bytes(b"another native library")
        with self.assertRaisesRegex(ValueError, "native dependency changed"):
            collector.check_files(binding)
        binary.write_bytes(b"rebuilt in place")
        with self.assertRaisesRegex(ValueError, "binary does not match"):
            collector.binary_identity(binary, metadata, TARGET)

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


if __name__ == "__main__":
    unittest.main()
