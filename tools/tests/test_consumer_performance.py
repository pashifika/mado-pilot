"""Deterministic scalar, retained-schema and cross-phase failure contracts; no native launches."""

import contextlib
import copy
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "consumer_performance", Path(__file__).resolve().parents[1] / "consumer-performance.py")
collector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(collector)

IMAGE_SPEC = importlib.util.spec_from_file_location(
    "consumer_binary_fixtures", Path(__file__).with_name("test_macos_image_observer.py"))
binary_fixtures = importlib.util.module_from_spec(IMAGE_SPEC)
IMAGE_SPEC.loader.exec_module(binary_fixtures)


def plan_for(root, case="trial-hit-roi"):
    root = Path(root).resolve()
    files = {}
    for name in ("engine", "driver", "fixture.json", "environment.json"):
        path = root / name
        path.write_bytes((name + "\n").encode())
        files[str(path)] = {"sha256": collector.sha256(path)}
    profile = {"opt_level": "0", "debuginfo": 2, "debug_assertions": True}
    binary = lambda name: {"path": str(root / name), "sha256": files[str(root / name)]["sha256"],
                           "cargo_artifact": {"profile": profile.copy()}}
    role = {"argv": [str(root / "driver"), "--engine", str(root / "engine"),
                     "--fixture", str(root / "fixture.json"), "--case", case[6:],
                     "--environment", str(root / "environment.json")],
            "engine": binary("engine"), "root_binary": binary("driver")}
    host = collector.host_identity()
    target = host["target"] or collector.component.TARGETS[0]
    return {"format": "actual-consumer-plan-v1", "execution_authorized": True, "case": case,
            "target": target, "host": host, "configuration": copy.deepcopy(collector.CONFIGURATION),
            "root": str(root), "campaign": str(root / "campaign"), "inputs": files,
            "roles": {"baseline": role}, "env": {"MADO_P050_OBSERVATIONS": "1"},
            "collector": collector.collector_identity(), "bounds": collector.LIMITS.copy(),
            "loader": False, "fixture": {"corpus_sha256": "f" * 64},
            "admission": None, "calibration": None, "profile": None}


def thread_point():
    return {"supported": True, "configured_threads": 10, "active_concurrency": None,
            "during_workload_invariance_observed": False, "started_us": 1, "finished_us": 2}


def trial_output(plan, *, role_name="baseline"):
    role = plan["roles"][role_name]
    parent = {"format": "trial-supervision-marks-v1", "epoch": "parent-control.started", "unit": "ns",
              "spawn_started_ns": 10, "spawn_returned_ns": 20, "child_started_received_ns": 30,
              "entry_settled_received_ns": 35, "terminal_received_ns": 40,
              "root_wait_returned_ns": 45, "streams_joined_ns": 50}
    child = {"version": 1, "operation": "recognition_trial", "primary": None, "child_pid": 102,
             "child_reaped": True, "forced": False, "exit_code": 0,
             "cleanup": {"clean": True, "session_closed": True, "status": "CleanupFinished", "close_error": None},
             "build": {"executable_sha256": role["engine"]["sha256"], "target": plan["target"]},
             "supervision_observation": parent.copy(), "environment_identity": "a" * 64,
             "identity": {"owner": "saved-frame", "revision": "test"},
             "result": {"kind": "template", "id": "p050-template-0", "threshold": 0.9, "outcome": "matched",
                        "matches": [{"bounds": {"x": 80, "y": 64, "width": 12, "height": 10}, "score": 1.0}]},
             "measurement": {"format": "trial-stage-marks-v1", "epoch": "control.started", "unit": "ns",
                             "unavailable": [], "names": list(collector.TRIAL_MARKS), "marks": list(range(19)),
                             "native_threads_after": thread_point(),
                             "process_rss_after": {"pid": 102, "method": "native-self-rss-v1",
                                                   "point": "after_trial_owner_release",
                                                   "scope": "recognition-child-process-point", "epoch": "control.started",
                                                   "clock_unit": "ns", "available": True, "reason": None,
                                                   "native_os_error": None, "started_ns": 19, "finished_ns": 20,
                                                   "rss_bytes": 100 * collector.MIB}}}
    probe = copy.deepcopy(child)
    probe.update(child_pid=101, operation="recognition_capabilities", measurement=None, result=None)
    fixture = {"id": plan["case"][6:], "max_results": 8, "threshold": 0.9, "search_region": [64, 48, 320, 180],
               "frame": {"width": 1920, "height": 1080, "bytes": 1920 * 1080 * 4, "sha256": "b" * 64},
               "template": {"width": 12, "height": 10, "sha256": "c" * 64}}
    return {"version": 1, "scope": "supervised-saved-rgba-template-trial", "case": plan["case"][6:],
            "corpus_sha256": plan["fixture"]["corpus_sha256"], "success": True, "status": "semantic_pass",
            "apparatus_failures": [], "oracle": {"passed": True, "violations": []},
            "ui_coverage": False, "native_capture": False, "native_input": False, "fixture_case": fixture,
            "inputs": {"fixture": {"path": role["argv"][4], **plan["inputs"][role["argv"][4]]},
                       "environment": {"path": role["argv"][8], **plan["inputs"][role["argv"][8]]},
                       "frame": {"sha256": "b" * 64}, "template": {"sha256": "c" * 64}},
            "operation": {"accepted": True, "cancellation_requested": False, "stop_error": None,
                          "ordinary_logs_omitted": 0, "shutdown": {"completed": True, "error": None},
                          "terminal_observed": True, "requires_parent_owned_tree_cleanup": False,
                          "identity": child["identity"], "run": "run-100", "total_duration_us": 1000,
                          "controller": {"state": "terminal", "error": None, "dropped_logs": 0,
                                         "run": "run-100", "operation": "recognition_trial",
                                         "progress": [{"event": "ChildStarted"}, {"event": "ChildStarted"}],
                                         "result": child, "attempts": [probe]}},
            "bounds": {"outer_primary_ms": 60000, "outer_cleanup_ms": 5000, "outer_output_bytes": collector.MIB}}


def record_for(output):
    return {"process_id": 100, "exit_code": 0, "timed_out": False, "output_limited": False,
            "cleanup_ok": True, "launch_error": None, "duration_seconds": 0.2,
            "stdout": collector.document(output).decode(), "stderr": "", "process_accounting": None,
            "cpu": {"user_seconds": 0.1, "system_seconds": 0.02, "status": "partial", "final": True,
                    "scope": "root-and-native-descendant-rollup", "source": "posix-wait4",
                    "reason": "descendant_rollup_unproven",
                    "accounting_resolution_seconds": None, "source_unit_seconds": 0.000001}}


def deadline_output(plan):
    names = ["observe", "release", "observe", "release", "log", "observe",
             "recognize", "log", "query", "release", "log", "query_wait"]
    records = [[collector.CALL_NAMES.index(name), index * 10000, index * 10000 + 2000, name != "query_wait"]
               for index, name in enumerate(names)]
    metrics = {}
    for name in collector.CALL_NAMES:
        count = names.count(name)
        metrics[name] = {"count": count, "failures": int(name == "query_wait"),
                         "total_us": count * 2, "max_us": 2 if count else 0}
    logs = [
        {"stage": "readiness", "frame": 1},
        {"stage": "confirmed-no-match", "frame": 2, "matched": False},
        {"stage": "query-wait-enter", "caller_observation_released": True,
         "source_frame": 2, "timeout_ms": 1},
    ]
    for row in logs:
        row.update(variant="roi", query_kind="deadline", corpus_sha256=plan["fixture"]["corpus_sha256"])
    return {"version": 1, "runs": [{
        "status": "FAIL", "exit_code": 0, "forced": False,
        "primary": {"category": "Timeout", "context": {"cause": {"status": "deadline_exceeded"}}},
        "build": {"executable_sha256": plan["roles"]["baseline"]["engine"]["sha256"], "target": plan["target"]},
        "cleanup": {"clean": True, "status": "CleanupFinished", "remaining": {"live_handles": 0},
                    "engine": {"session_closed": True, "clean": True, "close_error": None, "physical_cleanup_confirmed": True}},
        "observations": {"child_reaped": True, "startup_settled": True, "terminal_accounted": True,
                         "attempt_owners": 0, "live_handles": 0, "in_flight_native": 0, "queue_depth": 0,
                         "dropped_logs": 0, "recognitions": 1,
                         "logs": [collector.document(row).decode() for row in logs],
                         "native_threads_after": thread_point(), "operation_metrics": metrics,
                         "operation_samples": {"format": "host-call-records-v1", "epoch": "control.started",
                                               "unit": "ns", "dropped": 0, "capacity": 64,
                                               "names": list(collector.CALL_NAMES), "records": records}},
        "metrics": {name: 1000 for name in ("startup_us", "elapsed_us", "preflight_us", "workflow_us", "cleanup_us")},
        "milestones": [{"event": "ChildStarted", "pid": 102}],
    }]}


def cohort_rows(stage, metric="operation_us", baseline=1000, other=1000):
    return [{"slot": slot, "metrics": {metric: baseline if slot["role"] == "baseline" else other},
             "cpu_resolution": "1/1000"} for slot in collector.schedule(stage)]


RSS = "recognition_child_rss_bytes"
# Fixture values only; the collector binds whatever the reviewed policy declares.
HARD, FLOOR = 160 * collector.MIB, 2 * collector.MIB


def resident_contract(**changes):
    return {"unit": "bytes", "domain": "resident", "missing_reason": None,
            "api": "libc::proc_pidinfo(PROC_PIDTASKINFO).pti_resident_size",
            "owner": "actual recognition child", "interval": "one post-owner-release point",
            "hard_ceiling_bytes": HARD, "comparison_allowance_floor_bytes": FLOOR, **changes}


def resident_policy(**changes):
    return {RSS: {"hard_ceiling_bytes": HARD, "comparison_allowance_floor_bytes": FLOOR,
                  "justification": "fixture engineering budget, not a host capacity claim", **changes}}


def calibrated_resident(values=(100 * collector.MIB,) * 6, **changes):
    rows = cohort_rows("calibration", RSS)
    for row, value in zip(rows, values):
        row["metrics"][RSS] = value
        row["metrics"][OP] = 1000
    review = {"metrics": {OP: trial_contract(), RSS: resident_contract(**changes)}}
    return collector.derive_envelopes(rows, review, "trial-hit-roi")[RSS]


def paired_rows(stage, metric, timing, resident):
    """Balanced rows carrying one timing endpoint and the resident point per role."""
    rows = cohort_rows(stage, metric, *timing)
    for row in rows:
        row["metrics"][RSS] = resident[0] if row["slot"]["role"] == "baseline" else resident[1]
    return rows


OP = "operation_us"
# Synthetic fixture budget only: never a production default or an accepted user-derived budget.
TRIAL_CEILING, TRIAL_REGRESSION = 1200, 10


def trial_budget(**changes):
    return {"policy": collector.TRIAL_POLICY, "population": collector.TRIAL_POPULATION,
            "hard_ceiling_us": TRIAL_CEILING, "max_regression_us": TRIAL_REGRESSION, **changes}


def trial_contract(**changes):
    return {"api": "driver Instant", "owner": "driver", "interval": "operation through shutdown",
            "unit": "us", "missing_reason": None, "quantum": 1, "resolution": 1,
            "resolution_method": "source-bound", **trial_budget(**changes)}


def trial_declaration(**changes):
    return {**trial_budget(), "justification": "synthetic fixture budget, not a user-derived response requirement",
            **changes}


def calibrated_trial(values=(1000,) * 6, **changes):
    rows = cohort_rows("calibration")
    for row, value in zip(rows, values):
        row["metrics"][OP] = value
    return collector.derive_envelopes(rows, {"metrics": {OP: trial_contract(**changes)}}, "trial-hit-roi")[OP]


class ScalarRules(unittest.TestCase):
    def test_deadline_requires_query_wait_nonregression_without_speedup(self):
        rows = cohort_rows("calibration", "query_wait_ns")
        for case in ("deadline-full", "deadline-roi"):
            with self.subTest(case=case):
                with self.assertRaisesRegex(ValueError, "required graded metric missing"):
                    collector.derive_envelopes(rows, {"metrics": {}}, case)
                review = {"metrics": {"query_wait_ns": {"unit": "ns", "quantum": 1, "resolution": 1}}}
                envelopes = collector.derive_envelopes(rows, review, case)
                equal = cohort_rows("comparison", "query_wait_ns")
                self.assertEqual(collector.grade_rows(equal, "comparison", envelopes, case), [])
                slower = cohort_rows("comparison", "query_wait_ns", baseline=1000, other=1003)
                self.assertTrue(any("exceeds frozen ceiling" in row["reason"] for row in
                                    collector.grade_rows(slower, "comparison", envelopes, case)))

    def test_exact_quantum_rounding_without_float_loss(self):
        envelope = collector.scalar_envelope(["9007199254740993"] * 6, "1/10", "1/100")
        self.assertEqual(envelope["ceiling"], "90071992547409931/10")
        self.assertEqual(envelope["allowance"], "1/50")

    def test_noise_equality_is_accepted_but_larger_spread_is_not(self):
        self.assertEqual(collector.scalar_envelope([925, 1000, 1000, 1000, 1000, 1075], 1, 1)["spread"], "150")
        with self.assertRaisesRegex(ValueError, "noise"):
            collector.scalar_envelope([924, 1000, 1000, 1000, 1000, 1075], 1, 1)

    def test_historical_apple_trial_values_remain_old_scalar_nonpass(self):
        # Retained Apple trial-hit-roi calibration (PR #109 dossier): old arithmetic only, never regraded.
        values = [551859, 471323, 475221, 470318, 469603, 473136]
        spread, center = max(values) - min(values), collector.median(values)
        self.assertEqual((spread, center), (82256, collector.Fraction(944459, 2)))
        self.assertEqual(spread / max(center, 2), collector.Fraction(164512, 944459))
        with self.assertRaisesRegex(ValueError, "calibration noise exceeds 0.15"):
            collector.scalar_envelope(values, 1, 1)

    def test_single_violation_survives_good_median_and_missing_metric(self):
        envelopes = {"group_span_ns": collector.scalar_envelope([1000] * 6, 1, 1),
                     "recognize_0_ns": collector.scalar_envelope([1] * 6, 1, 1)}
        rows = cohort_rows("comparison", "group_span_ns", baseline=1000, other=900)
        rows[0]["metrics"]["group_span_ns"] = 1003
        issues = collector.grade_rows(rows, "comparison", envelopes, "group-full")
        self.assertTrue(any("exceeds frozen ceiling" in row["reason"] for row in issues))
        self.assertTrue(any(row["category"] == "unavailable" for row in issues))

    def test_unchanged_role_bias_and_strict_group_and_query_benefit_threshold(self):
        for case, metric in (("group-full", "group_span_ns"), ("matched-roi", "query_wait_ns")):
            with self.subTest(case=case):
                self.assertEqual(collector.benefit_metric(case), metric)
                envelope = {metric: collector.scalar_envelope([1000] * 6, 1, 1)}
                rows = cohort_rows("control", metric, baseline=1000, other=997)
                self.assertTrue(any("role bias" in row["reason"] for row in
                                    collector.grade_rows(rows, "control", envelope, case)))
                for other in (1000, 998):
                    rows = cohort_rows("comparison", metric, baseline=1000, other=other)
                    self.assertIn(collector.issue("numerical", metric + ": scalar benefit does not exceed allowance"),
                                  collector.grade_rows(rows, "comparison", envelope, case))
                rows = cohort_rows("comparison", metric, baseline=1000, other=997)
                self.assertEqual(collector.grade_rows(rows, "comparison", envelope, case), [])

    def test_order_and_chronological_drift_are_independent(self):
        envelope = {"group_span_ns": collector.scalar_envelope([1000] * 6, 1, 1)}
        rows = cohort_rows("control", "group_span_ns")
        for row in rows:
            if row["slot"]["position"] == 1:
                row["metrics"]["group_span_ns"] = 990
        self.assertTrue(any("order" in row["reason"] for row in collector.grade_rows(rows, "control", envelope, "group-full")))
        rows = cohort_rows("control", "group_span_ns")
        for row in rows:
            if row["slot"]["process"] >= 3:
                row["metrics"]["group_span_ns"] = 990
        self.assertTrue(any("drift" in row["reason"] for row in collector.grade_rows(rows, "control", envelope, "group-full")))

    def test_cpu_cannot_become_a_graded_or_frozen_metric(self):
        rows = cohort_rows("calibration", "lifetime_cpu_seconds", 1, 1)
        review = {"metrics": {"lifetime_cpu_seconds": {"unit": "s", "quantum": 1, "resolution": 1}}}
        with self.assertRaises(ValueError):
            collector.derive_envelopes(rows, review, "replay-full")
        with self.assertRaises(ValueError):
            collector.grade_rows(cohort_rows("control", "lifetime_cpu_seconds"), "control",
                                 {"lifetime_cpu_seconds": collector.scalar_envelope([1] * 6, 1, 1)},
                                 "replay-full")
        for case in collector.CASES:
            self.assertNotIn("lifetime_cpu_seconds", collector.selectable_metrics(case))

    def test_six_twelve_twelve_and_balanced_positions(self):
        self.assertEqual([len(collector.schedule(stage)) for stage in ("calibration", "control", "comparison")], [6, 12, 12])
        rows = collector.schedule("comparison")
        self.assertEqual([row["role"] for row in rows[:4]], ["baseline", "candidate", "candidate", "baseline"])
        for role in ("baseline", "candidate"):
            selected = [row for row in rows if row["role"] == role]
            self.assertEqual([row["process"] for row in selected], list(range(6)))
            self.assertEqual(sum(row["position"] == 0 for row in selected), 3)

    def test_pair_reservation_and_pause_keep_original_deadline(self):
        smaller = {**collector.LIMITS, "primary_seconds": 60, "output_limit_bytes": collector.MIB}
        self.assertTrue(collector.admission_room(630, 900, 2, smaller))
        self.assertFalse(collector.admission_room(631, 900, 2, smaller))
        self.assertTrue(collector.admission_room(735, 900, 1, smaller))
        self.assertFalse(collector.admission_room(900, 900, 1, smaller))


class BoundReferences(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "review.json"
        self.binding = collector.save(self.path, {"accepted": False})

    def test_reference_parse_uses_verified_bytes_when_path_changes_after_digest(self):
        original_sha256 = collector.hashlib.sha256
        replacement = collector.document({"accepted": True})
        path = self.path

        class ReplacingDigest:
            def __init__(self, *args, **kwargs):
                self.value = original_sha256(*args, **kwargs)

            def update(self, data):
                self.value.update(data)

            def hexdigest(self):
                result = self.value.hexdigest()
                path.write_bytes(replacement)
                return result

        with mock.patch.object(collector.hashlib, "sha256", ReplacingDigest):
            result = collector.read_reference(self.binding)
        self.assertEqual(result, {"accepted": False})
        self.assertEqual(self.path.read_bytes(), replacement)

    def test_reference_byte_limit_includes_exact_boundary(self):
        size = self.path.stat().st_size
        self.assertEqual(collector.read_reference(self.binding, size), {"accepted": False})
        with self.assertRaises(ValueError):
            collector.read_reference(self.binding, size - 1)


class RetainedSchema(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.plan = plan_for(self.temporary.name)
        self.output = trial_output(self.plan)

    def reconcile(self, output=None, **changes):
        record = record_for(self.output if output is None else output)
        record.update(changes)
        return collector.reconcile(self.plan, collector.schedule("smoke")[0], record)

    def test_actual_trial_scalar_excludes_driver_preparation(self):
        self.output["total_duration_us"] = 999999
        result = self.reconcile()
        self.assertEqual(result["issues"], [])
        self.assertEqual(result["metrics"]["operation_us"], 1000)
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])
        self.assertEqual(result["raw_cpu_observation"]["user_plus_system_seconds"], "3/25")
        self.assertEqual(result["cpu_coverage"]["status"], "partial")
        self.assertEqual(result["cpu"]["status"], "partial")
        self.assertIsNone(result["cpu_resolution"])
        self.assertTrue(result["cpu_coverage"]["issues"])
        self.assertEqual(result["driver_declared_outer"], self.output["bounds"])

    def test_sampled_rss_absence_is_not_zero_or_semantic_failure(self):
        self.output["operation"]["controller"]["result"]["measurement"].pop("process_rss_after")
        result = self.reconcile()
        self.assertEqual(result["issues"], [])
        self.assertNotIn("recognition_child_rss_bytes", result["metrics"])

    def test_retained_inspection_is_read_only_and_never_calibration(self):
        root = Path(self.temporary.name)
        plan_path, raw_path = root / "plan.json", root / "retained.json"
        collector.save(plan_path, self.plan)
        collector.save(raw_path, {"slot": {"role": "baseline", "case": "trial-hit-roi-rss-on"},
                                  "record": record_for(self.output)})
        before = raw_path.read_bytes()
        result = collector.inspect_retained(plan_path, raw_path)
        self.assertFalse(result["qualification"])
        self.assertEqual(result["kind"], "retained-development-inspection")
        self.assertEqual(result["issues"], [])
        self.assertEqual(result["observation"]["metrics"]["operation_us"], 1000)
        self.assertEqual(raw_path.read_bytes(), before)
        self.assertTrue(result["admission_issues"])

    def test_original_timeout_and_cleanup_failures_survive_missing_cpu(self):
        result = self.reconcile(timed_out=True, cleanup_ok=False, cpu={"status": "unknown", "final": False})
        self.assertIn("cleanup", {row["category"] for row in result["issues"]})
        self.assertTrue(any("deadline" in row["reason"] for row in result["issues"]))
        self.assertTrue(result["unavailable"])
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])

    def test_capability_cleanup_and_binary_identity_are_enforced(self):
        self.output["operation"]["controller"]["attempts"][0]["child_reaped"] = False
        self.assertEqual(self.reconcile()["issues"][0]["category"], "cleanup")
        self.output = trial_output(self.plan)
        self.output["operation"]["controller"]["result"]["build"]["executable_sha256"] = "0" * 64
        self.assertEqual(self.reconcile()["issues"][0]["category"], "identity")

    def test_oracle_boolean_cannot_hide_wrong_public_match(self):
        self.output["operation"]["controller"]["result"]["result"]["matches"][0]["bounds"]["x"] = 81
        self.assertTrue(any("oracle" in row["reason"] for row in self.reconcile()["issues"]))

    def test_stage_names_and_parent_epoch_are_not_interchangeable(self):
        self.output["operation"]["controller"]["result"]["measurement"]["names"][4] = "not_initialized"
        self.assertTrue(self.reconcile()["issues"])
        self.output = trial_output(self.plan)
        self.output["operation"]["controller"]["result"]["supervision_observation"]["epoch"] = "control.started"
        self.assertTrue(any("epoch" in row["reason"] for row in self.reconcile()["issues"]))

    def test_after_release_rss_point_needs_same_pid_and_bracket(self):
        rss = self.output["operation"]["controller"]["result"]["measurement"]["process_rss_after"]
        rss["pid"] = 101
        self.assertEqual(self.reconcile()["issues"][0]["category"], "identity")
        rss["pid"], rss["started_ns"] = 102, 0
        self.assertTrue(any("bracket" in row["reason"] for row in self.reconcile()["issues"]))

    def test_driver_declared_envelope_must_fit_actual_enforcement(self):
        self.plan["bounds"]["primary_seconds"] = 60
        self.plan["bounds"]["output_limit_bytes"] = collector.MIB
        self.assertEqual(self.reconcile()["issues"], [])
        self.plan["bounds"]["primary_seconds"] = 59
        self.assertTrue(any("driver-declared" in row["reason"] for row in self.reconcile()["issues"]))

    def test_every_reviewed_trial_operation_obeys_declared_ceiling(self):
        def issues(**changes):
            review = {"metrics": {OP: trial_contract(**changes)}}
            result = collector.reconcile(self.plan, collector.schedule("smoke")[0], record_for(self.output), review)
            return result["issues"], [row for row in result["unavailable"] if row["reason"].startswith(OP)]

        self.assertEqual(issues(hard_ceiling_us=1000), ([], []))
        self.assertEqual(issues(hard_ceiling_us=999),
                         ([collector.issue("numerical", OP + " exceeds fixed trial responsiveness ceiling")], []))
        # A malformed budget is missing method evidence, never a passed or failed observation.
        failures, missing = issues(hard_ceiling_us=True)
        self.assertEqual(failures, [])
        self.assertTrue(missing)

    def test_trial_ceiling_survives_later_result_or_resource_failure(self):
        for failure in ("result", "driver-semantic", "resource"):
            with self.subTest(failure=failure):
                output = copy.deepcopy(self.output)
                output["operation"]["total_duration_us"] = TRIAL_CEILING + 1
                child = output["operation"]["controller"]["result"]
                if failure in ("result", "driver-semantic"):
                    child["result"]["matches"][0]["bounds"]["x"] = 81
                else:
                    child["measurement"]["process_rss_after"]["started_ns"] = 0
                record = record_for(output)
                if failure == "driver-semantic":
                    output.update(success=False, status="semantic_failure",
                                  oracle={"passed": False, "violations": ["public_match_bounds_mismatch"]})
                    record = record_for(output)
                    record["exit_code"] = 1
                result = collector.reconcile(self.plan, collector.schedule("smoke")[0], record,
                                             {"metrics": {OP: trial_contract()}})
                self.assertEqual(result["metrics"][OP], TRIAL_CEILING + 1)
                self.assertTrue({"semantics", "numerical"} <= {row["category"] for row in result["issues"]})
                if failure == "driver-semantic":
                    self.assertIn("execution", {row["category"] for row in result["issues"]})
                    self.assertEqual(result["application_status"], "semantic_failure")
                    self.assertEqual(result["original_exit_code"], 1)

    def test_unbound_child_duration_is_not_a_trial_budget_observation(self):
        self.output["operation"]["total_duration_us"] = TRIAL_CEILING + 1
        self.output["operation"]["controller"]["result"]["build"]["executable_sha256"] = "0" * 64
        result = collector.reconcile(self.plan, collector.schedule("smoke")[0], record_for(self.output),
                                     {"metrics": {OP: trial_contract()}})
        self.assertIsNone(result["metrics"][OP])
        self.assertIn("identity", {row["category"] for row in result["issues"]})
        self.assertNotIn("numerical", {row["category"] for row in result["issues"]})

    def test_semantic_failure_does_not_mask_sticky_cleanup_identity(self):
        self.output["success"] = False
        self.output["operation"]["controller"]["result"]["cleanup"]["clean"] = False
        self.output["operation"]["controller"]["result"]["build"]["executable_sha256"] = "0" * 64
        result = self.reconcile()
        categories = {row["category"] for row in result["issues"]}
        self.assertTrue({"semantics", "cleanup", "identity"} <= categories)

    def test_expected_deadline_contract_never_relabels_application_pass(self):
        self.plan["case"] = "deadline-roi"
        result = self.reconcile(deadline_output(self.plan), exit_code=1)
        self.assertEqual(result["issues"], [])
        self.assertEqual(result["application_status"], "FAIL")
        self.assertEqual(result["original_exit_code"], 1)
        self.assertEqual(result["primary"]["context"]["cause"]["status"], "deadline_exceeded")
        self.assertIsNone(collector.benefit_metric("deadline-roi"))
        self.assertTrue(result["unavailable"])

    def test_deadline_root_one_engine_zero_are_independent_contracts(self):
        self.plan["case"] = "deadline-roi"
        output = deadline_output(self.plan)
        self.assertTrue(self.reconcile(output, exit_code=0)["issues"])
        output["runs"][0]["exit_code"] = 1
        result = self.reconcile(output, exit_code=1)
        self.assertTrue(any("engine-child exit" in row["reason"] for row in result["issues"]))
        self.assertEqual(result["application_status"], "FAIL")
        self.assertEqual(result["original_exit_code"], 1)

    def test_rejected_trial_reservation_with_clean_shutdown_is_not_cleanup_failure(self):
        self.output.update(success=False, status="semantic_fail")
        self.output["operation"] = {"accepted": False, "shutdown": {"completed": True, "error": None},
                                    "controller": {"state": "idle", "attempts": [], "result": None}}
        result = self.reconcile()
        self.assertTrue(result["issues"])
        self.assertNotIn("cleanup", {row["category"] for row in result["issues"]})

    def test_deadline_application_fail_is_retained_when_other_fields_are_missing(self):
        self.plan["case"] = "deadline-roi"
        fault = {"category": "Timeout", "context": {"cause": {"status": "deadline_exceeded"}}}
        output = {"version": 1, "runs": [{"status": "FAIL", "exit_code": 0, "primary": fault}]}
        result = self.reconcile(output, exit_code=1)
        self.assertEqual(result["application_status"], "FAIL")
        self.assertEqual(result["primary"], fault)
        self.assertEqual(result["original_exit_code"], 1)
        self.assertTrue(result["issues"])

    def test_cpu_units_and_job_totals_never_imply_resolution_or_census(self):
        record = record_for(self.output)
        record["cpu"].update(status="complete", reason=None,
                             source="windows-job-basic-accounting", scope="owned-job-including-nested-jobs",
                             source_unit_seconds=0.0000001)
        record["process_accounting"] = {"total_processes": 5, "active_processes": 0, "terminated_processes": 0}
        result = collector.reconcile(self.plan, collector.schedule("smoke")[0], record)
        self.assertIsNone(result["cpu_resolution"])
        self.assertTrue(result["cpu_coverage"]["issues"])
        self.assertNotIn("owned_processes", result)
        self.assertTrue(result["unavailable"])

    def test_malformed_cpu_does_not_hide_wall_observation_or_pass_semantics(self):
        original = record_for(self.output)["cpu"]
        cases = [None, [], {**original, "status": "invented"}, {**original, "final": 1},
                 {**original, "user_seconds": None}, {**original, "user_seconds": -1},
                 {**original, "system_seconds": True}, {**original, "system_seconds": "NaN"},
                 {**original, "system_seconds": "1/0"}, {**original, "source_unit_seconds": 0},
                 {**original, "accounting_resolution_seconds": -1},
                 {**original, "reason": None}, {**original, "scope": ""},
                 {**original, "status": "complete", "final": False, "reason": None},
                 {**original, "status": "complete", "reason": None},
                 {**original, "source": "windows-job-basic-accounting"},
                 {**original, "source": "unrecognized-accounting"}]
        for cpu in cases:
            with self.subTest(cpu=cpu):
                result = self.reconcile(cpu=cpu)
                self.assertEqual(result["metrics"][OP], 1000)
                self.assertEqual(result["metrics"]["invocation_elapsed_seconds"], "1/5")
                self.assertIn("semantics", {problem["category"] for problem in result["issues"]})

    def test_malformed_wall_timing_is_independent_of_valid_unknown_cpu(self):
        cpu = {**record_for(self.output)["cpu"], "status": "unknown", "final": False,
               "user_seconds": None, "system_seconds": None, "reason": "wait4_unavailable"}
        result = self.reconcile(cpu=cpu, duration_seconds=-1)
        self.assertIn("semantics", {problem["category"] for problem in result["issues"]})
        self.assertEqual(result["cpu"], cpu)

    def test_loader_transport_validates_pid_and_owned_path_without_claiming_closure(self):
        self.plan["loader"] = True
        self.plan["target"] = collector.component.TARGETS[0]
        sinks = Path(self.temporary.name) / "sinks"
        sinks.mkdir()
        reports = []
        for pid in (101, 102):
            path = sinks / (str(pid) + ".stderr")
            payload = f"dyld[{pid}]: /runtime/image.dylib\n".encode()
            path.write_bytes(payload)
            reports.append({
                "loader_enabled": True, "child_pid": pid, "complete": True, "eof": True,
                "foreign_lines": 0, "interleaved": 0, "malformed_prefixes": 0, "oversized_lines": 0,
                "lines": 1, "sink": {"path": str(path), "bytes_written": len(payload),
                                    "truncated": False, "write_error": None},
            })
        record = {"process_id": 100, "stderr": "dyld[100]: /runtime/image.dylib\n"}
        retained = collector.loader_check(self.plan, {"children": reports}, record, sinks)
        self.assertEqual(len(retained), 2)
        self.assertTrue(all(row["scope"] == "transport-only-not-image-closure" for row in retained))
        (sinks / "101.stderr").write_bytes(b"dyld[999]: /runtime/image.dylib\n")
        with self.assertRaisesRegex(ValueError, "PID transport"):
            collector.loader_check(self.plan, {"children": reports}, record, sinks)


class ImmutableCampaign(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.plan = plan_for(self.root)
        self.plan["host"]["target"] = self.plan["target"]
        self.enterContext(mock.patch.object(collector, "host_identity", return_value=self.plan["host"]))
        self.campaign = collector.Campaign(self.plan)
        self.output = trial_output(self.plan)
        self.campaign.initialize()

    def test_receipt_creation_never_replaces_failed_evidence(self):
        path = self.root / "profile.json"
        collector.save(path, {"failed": True})
        original = path.read_bytes()
        with self.assertRaises(FileExistsError):
            collector.save(path, {"failed": False})
        self.assertEqual(path.read_bytes(), original)

    def test_control_failure_fences_another_case_and_phase_owner(self):
        self.campaign.fence("control", "unchanged role bias", "trial-hit-roi-control")
        second = copy.deepcopy(self.plan)
        second["case"] = "trial-miss-full"
        owner = collector.Campaign(second)
        self.assertTrue(any(row["category"] == "control" for row in owner.fences()))

    def test_unfinished_phase_fences_new_owner_even_without_final_summary(self):
        collector.save(self.campaign.root / "phases" / "trial-hit-roi-calibration.json",
                       {"stage": "calibration", "output": str(self.root / "lost-owner")})
        fresh_owner = collector.Campaign(self.plan)
        self.assertTrue(any(row["category"] == "cleanup" for row in fresh_owner.fences()))

    def test_changed_sealed_summary_is_sticky_identity_failure(self):
        phase = self.root / "prior"
        phase.mkdir()
        seal = collector.save(phase / "summary.json", {"issues": []})
        collector.save(self.campaign.root / "phases" / "prior.json", {"stage": "calibration", "output": str(phase)})
        collector.save(self.campaign.root / "phases" / "prior.complete", seal)
        (phase / "summary.json").write_text('{"issues":[],"altered":true}\n')
        self.assertTrue(any(row["category"] == "identity" for row in self.campaign.fences()))

    def test_missing_admission_retains_every_unrun_slot_without_launch(self):
        with mock.patch.object(collector, "run_process", side_effect=AssertionError("must not launch")):
            with contextlib.redirect_stdout(io.StringIO()):
                result = collector.collect(self.plan, "calibration", self.root / "calibration")
        self.assertFalse(result["qualification"])
        self.assertEqual(result["attempts"], [])
        self.assertEqual(result["unrun"], collector.schedule("calibration"))
        self.assertTrue(all(any(gate in row["reason"] for row in result["admission_issues"]) for gate in collector.GATES))
        self.assertTrue((self.root / "calibration" / "summary.json").is_file())

    def test_failed_control_precedes_any_candidate_launch(self):
        self.plan["roles"]["candidate"] = copy.deepcopy(self.plan["roles"]["baseline"])
        with mock.patch.object(collector, "run_process", side_effect=AssertionError("candidate must not launch")):
            with contextlib.redirect_stdout(io.StringIO()):
                control = collector.collect(self.plan, "control", self.root / "control")
                result = collector.collect(self.plan, "comparison", self.root / "comparison")
        self.assertTrue(control["issues"])
        self.assertEqual(len(control["unrun"]), 12)
        self.assertEqual(result["attempts"], [])
        self.assertEqual(len(result["unrun"]), 12)
        self.assertTrue(any(row["category"] == "control" for row in result["issues"]))

    def test_existing_stage_cannot_be_retried_into_new_output(self):
        with mock.patch.object(collector, "run_process", side_effect=AssertionError("must not launch")):
            with contextlib.redirect_stdout(io.StringIO()):
                collector.collect(self.plan, "calibration", self.root / "first")
                original = (self.campaign.root / "phases" / "trial-hit-roi-calibration.complete").read_bytes()
                result = collector.collect(self.plan, "calibration", self.root / "second")
        self.assertEqual(result["attempts"], [])
        self.assertTrue(result["issues"])
        self.assertEqual((self.campaign.root / "phases" / "trial-hit-roi-calibration.complete").read_bytes(), original)

    def test_identity_guard_detects_changed_source_without_mutating_old_receipt(self):
        original = copy.deepcopy(self.plan["inputs"])
        (self.root / "engine").write_bytes(b"different executable")
        issues = collector.verify_inputs(self.plan, collector.time.monotonic() + 10)
        self.assertTrue(any(row["category"] == "identity" for row in issues))
        self.assertEqual(self.plan["inputs"], original)

    def test_expired_verification_is_unavailable_not_a_false_identity_fence(self):
        failures = collector.verify_inputs(self.plan, collector.time.monotonic() - 1)
        self.assertTrue(any("verification incomplete" in row["reason"] for row in failures))
        self.assertNotIn("identity", {row["category"] for row in failures})
        for row in failures:
            if row["category"] in ("identity", "cleanup"):
                self.campaign.fence(row["category"], row["reason"], "expired")
        self.assertEqual(self.campaign.fences(), [])

    def test_slow_durable_start_cannot_launch_after_retention_budget_expires(self):
        clock = [100.0]
        durable_save = collector.save

        def delayed_start(path, value, maximum=4 * collector.MIB):
            binding = durable_save(path, value, maximum)
            if str(path).endswith(".started.json"):
                clock[0] += 11
            return binding

        with mock.patch.object(collector.time, "monotonic", side_effect=lambda: clock[0]), \
                mock.patch.object(collector, "save", side_effect=delayed_start), \
                mock.patch.object(collector, "run_process", side_effect=AssertionError("must not launch")):
            with contextlib.redirect_stdout(io.StringIO()):
                result = collector.collect(self.plan, "smoke", self.root / "late-start")
        self.assertEqual(result["attempts"][0]["state"], "not_launched")
        self.assertIsNone(result["attempts"][0]["raw"])
        self.assertTrue(any(row["category"] == "deadline" for row in result["issues"]))
        self.assertNotIn("cleanup", {row["category"] for row in result["issues"]})
        self.assertEqual(self.campaign.fences(), [])

    def test_campaign_is_bound_but_runtime_bundle_does_not_hash_itself(self):
        moved = {**self.plan, "campaign": str(self.root / "other-campaign")}
        self.assertNotEqual(collector.context_digest(moved), collector.context_digest(self.plan))
        self.assertNotEqual(collector.runtime.execution_binding(moved), collector.runtime.execution_binding(self.plan))
        closed = {**self.plan, "runtime_closure": {"path": "/closure", "sha256": "a" * 64}}
        self.assertNotEqual(collector.context_digest(closed), collector.context_digest(self.plan))
        self.assertEqual(collector.runtime.execution_binding(closed), collector.runtime.execution_binding(self.plan))

    def test_active_phase_refusal_keeps_full_unrun_receipt_and_existing_lock(self):
        active = self.campaign.root / "active"
        collector.save(active, {"pid": 123})
        before = active.read_bytes()
        with mock.patch.object(collector, "run_process", side_effect=AssertionError("must not launch")):
            with contextlib.redirect_stdout(io.StringIO()):
                result = collector.collect(self.plan, "smoke", self.root / "locked-phase")
        self.assertEqual(result["unrun"], collector.schedule("smoke"))
        self.assertEqual(active.read_bytes(), before)
        self.assertTrue((self.root / "locked-phase" / "summary.json").is_file())

    def test_noisy_trial_calibration_is_retained_inconclusive_not_numerical(self):
        # Method, launch and per-invocation boundaries are substituted; schedule, receipts and derivation are real.
        self.plan["roles"]["candidate"] = copy.deepcopy(self.plan["roles"]["baseline"])
        review = {"metrics": {OP: trial_contract()}}
        observation = collector.reconcile(self.plan, collector.schedule("smoke")[0], record_for(self.output))
        values = iter([1000, 1000 + TRIAL_REGRESSION + 1, 1000 + TRIAL_REGRESSION + 1, 1000, 1000, 1000])

        def observed(*_):
            return {**copy.deepcopy(observation), "metrics": {OP: next(values)}, "issues": [], "unavailable": []}

        with mock.patch.object(collector, "admission", side_effect=lambda plan: (review, [])), \
                mock.patch.object(collector, "reconcile", side_effect=observed), \
                mock.patch.object(collector, "run_process", return_value=record_for({})) as launch:
            with contextlib.redirect_stdout(io.StringIO()):
                result = collector.collect(self.plan, "calibration", self.root / "calibration")
        self.assertEqual((launch.call_count, result["unrun"], result["envelopes"]), (6, [], {}))
        self.assertFalse(result["qualification"])
        self.assertEqual([row["category"] for row in result["issues"]], ["inconclusive"])
        self.assertEqual(self.campaign.fences(), [])

    def test_observation_off_and_non_dev_role_refused(self):
        bad = copy.deepcopy(self.plan)
        bad["env"]["MADO_P050_OBSERVATIONS"] = "0"
        with self.assertRaisesRegex(ValueError, "observation-on"):
            collector.validate_plan(bad)
        bad = copy.deepcopy(self.plan)
        bad["roles"]["baseline"]["engine"]["cargo_artifact"]["profile"]["opt_level"] = "3"
        with self.assertRaisesRegex(ValueError, "dev build"):
            collector.validate_plan(bad)

    def failed_control(self, *, early=False):
        self.plan["roles"]["candidate"] = copy.deepcopy(self.plan["roles"]["baseline"])
        review = {"metrics": {OP: trial_contract()}}
        slots = iter(collector.schedule("control"))
        original_reconcile = collector.reconcile

        def launch(*args, **kwargs):
            slot = next(slots)
            output = copy.deepcopy(self.output)
            output["operation"]["total_duration_us"] = (TRIAL_CEILING + 1 if early else
                1000 if slot["role"] == "baseline" else 1000 + TRIAL_REGRESSION + 1)
            return record_for(output)

        def reconcile(*args, **kwargs):
            result = original_reconcile(*args, **kwargs)
            # Only optional qualification accounting is substituted; semantics and grading are real.
            result["unavailable"] = []
            return result

        with mock.patch.object(collector, "admission", return_value=(review, [])), \
                mock.patch.object(collector, "prerequisites", return_value=({OP: calibrated_trial()}, [])), \
                mock.patch.object(collector, "reconcile", side_effect=reconcile), \
                mock.patch.object(collector, "run_process", side_effect=launch) as process, \
                contextlib.redirect_stdout(io.StringIO()):
            result = collector.collect(self.plan, "control", self.root / "control")
        self.assertEqual(process.call_count, 1 if early else 12)
        self.assertTrue(result["issues"])
        self.assertFalse(result["qualification"])
        self.assertEqual(result["qualification_status"], "failed")
        return result

    def test_numerical_control_preserves_failed_evidence_but_allows_distinct_smokes(self):
        self.failed_control()
        paths = [*self.campaign.root.rglob("*"), *(self.root / "control").rglob("*")]
        retained = {path: path.read_bytes() for path in paths if path.is_file()}
        with mock.patch.object(collector, "run_process", return_value=record_for(self.output)) as launch, \
                contextlib.redirect_stdout(io.StringIO()):
            first = collector.collect(self.plan, "smoke", self.root / "diagnostic-one")
            second = collector.collect(self.plan, "smoke", self.root / "diagnostic-two")
        self.assertEqual(launch.call_count, 2)
        for result in (first, second):
            self.assertEqual((result["issues"], result["unrun"]), ([], []))
            self.assertEqual((result["execution_status"], result["qualification_status"]),
                             ("completed", "not-requested"))
            self.assertFalse(result["qualification"])
        self.assertNotEqual(first["registration"], second["registration"])
        self.assertEqual(retained, {path: path.read_bytes() for path in retained})
        self.assertTrue(self.campaign.fences())
        self.assertEqual(self.campaign.fences(diagnostic=True), [])
        with mock.patch.object(collector, "run_process") as launch, contextlib.redirect_stdout(io.StringIO()):
            refused = collector.collect(self.plan, "comparison", self.root / "comparison")
        launch.assert_not_called()
        self.assertTrue(refused["issues"])

    def test_numerical_early_stop_allows_smoke_without_resuming_unrun_control(self):
        control = self.failed_control(early=True)
        retained = (self.root / "control/summary.json").read_bytes()
        self.assertEqual(len(control["unrun"]), 11)
        with mock.patch.object(collector, "run_process", return_value=record_for(self.output)) as launch, \
                contextlib.redirect_stdout(io.StringIO()):
            result = collector.collect(self.plan, "smoke", self.root / "diagnostic")
        self.assertEqual((launch.call_count, result["issues"]), (1, []))
        self.assertEqual((self.root / "control/summary.json").read_bytes(), retained)

    def test_unbound_or_changed_control_cannot_bypass_fence(self):
        self.failed_control()
        path = self.root / "control/00.raw.json"
        value = collector.load(path)
        value["record"]["cleanup_ok"] = False
        path.write_bytes(collector.document(value))
        with mock.patch.object(collector, "run_process") as launch, contextlib.redirect_stdout(io.StringIO()):
            result = collector.collect(self.plan, "smoke", self.root / "diagnostic")
        launch.assert_not_called()
        self.assertIn("identity", {problem["category"] for problem in result["issues"]})

    def test_unknown_identity_and_cleanup_fences_refuse_new_smoke_output(self):
        self.plan["roles"]["candidate"] = copy.deepcopy(self.plan["roles"]["baseline"])
        for category in ("control", "identity", "cleanup", "unknown"):
            with self.subTest(category=category):
                plan = {**self.plan, "campaign": str(self.root / (category + "-campaign"))}
                owner = collector.Campaign(plan)
                owner.initialize()
                owner.fence(category, "unresolved original state", "unregistered")
                with mock.patch.object(collector, "run_process") as launch, contextlib.redirect_stdout(io.StringIO()):
                    result = collector.collect(plan, "smoke", self.root / category, smoke_role="candidate")
                launch.assert_not_called()
                self.assertEqual(result["attempts"], [])
                self.assertTrue(result["issues"])

    def test_unfinished_smoke_registration_refuses_an_independent_diagnostic(self):
        collector.save(self.campaign.root / "phases" / "unfinished-smoke.json",
                       {"stage": "smoke", "output": str(self.root / "unknown")})
        with mock.patch.object(collector, "run_process") as launch, contextlib.redirect_stdout(io.StringIO()):
            result = collector.collect(self.plan, "smoke", self.root / "diagnostic")
        launch.assert_not_called()
        self.assertIn("cleanup", {problem["category"] for problem in result["issues"]})

    def test_candidate_smoke_cli_describes_numeric_misses_and_report_keeps_original_validation(self):
        engine_path = self.root / "candidate-engine"
        engine_path.write_bytes(b"candidate engine\n")
        self.plan["inputs"][str(engine_path)] = {"sha256": collector.sha256(engine_path)}
        candidate = copy.deepcopy(self.plan["roles"]["baseline"])
        candidate["argv"][2] = str(engine_path)
        candidate["engine"].update(path=str(engine_path), sha256=collector.sha256(engine_path))
        self.plan["roles"]["candidate"] = candidate
        self.output = trial_output(self.plan, role_name="candidate")
        review = {"metrics": {OP: trial_contract(hard_ceiling_us=999),
                              RSS: resident_contract(hard_ceiling_bytes=99 * collector.MIB)}}
        plan_path = self.root / "selected-plan.json"
        collector.save(plan_path, self.plan)
        directory = self.root / "diagnostic"
        with mock.patch.object(collector, "admission", return_value=(review, [])), \
                mock.patch.object(collector, "run_process", return_value=record_for(self.output)) as launch:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                code = collector.main(["smoke", "--plan", str(plan_path), "--role", "candidate",
                                       "--output", str(directory)])
            self.assertEqual((code, launch.call_count), (0, 1))
            result = collector.load(directory / "summary.json")
            self.assertEqual(result["format"], collector.PHASE_FORMAT)
            self.assertEqual(result["schedule"][0]["role"], "candidate")
            self.assertEqual(set(result["metric_roles"]["graded"]), {OP, RSS})
            self.assertIn("lifetime_cpu_seconds", result["metric_roles"]["diagnostic"])
            self.assertNotIn("lifetime_cpu_seconds", result["metric_roles"]["graded"])
            self.assertEqual(result["cpu_diagnostics"][0]["cpu"], record_for(self.output)["cpu"])
            self.assertEqual(collector.load(directory / "00.started.json")["argv"], candidate["argv"])
            self.assertEqual(result["issues"], [])
            self.assertEqual(len(result["observation_issues"]), 2)
            validation = collector.load(directory / "00.validation.json")
            self.assertEqual(result["observation_issues"], validation["issues"])
            self.assertIsNone(validation["metrics"]["lifetime_cpu_seconds"])
            self.assertTrue(validation["unavailable"])
            original = {path: path.read_bytes() for path in directory.iterdir() if path.is_file()}
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(collector.main(["report", str(directory)]), 0)
            report = collector.strict_json(output.getvalue())
            self.assertEqual(report["rows"][0]["slot"]["role"], "candidate")
            self.assertEqual(report["observation_issues"], result["observation_issues"])
            self.assertEqual(report["rows"][0]["issues"], validation["issues"])
            self.assertEqual(report["cpu_diagnostics"], result["cpu_diagnostics"])
            self.assertEqual(report["metric_roles"], result["metric_roles"])
            self.assertEqual(report["qualification_status"], "not-requested")
            self.assertEqual(original, {path: path.read_bytes() for path in original})
            self.assertEqual(launch.call_count, 1)

    def test_missing_candidate_smoke_refuses_before_output_or_registration(self):
        plan_path = self.root / "selected-plan.json"
        collector.save(plan_path, self.plan)
        directory = self.root / "missing-candidate"
        with mock.patch.object(collector, "run_process") as launch, \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = collector.main(["smoke", "--plan", str(plan_path), "--role", "candidate",
                                   "--output", str(directory)])
        self.assertEqual(code, 2)
        launch.assert_not_called()
        self.assertFalse(directory.exists())
        self.assertEqual(list((self.campaign.root / "phases").iterdir()), [])

    def test_explicit_calibration_cli_still_fails_numeric_limit_without_retry(self):
        self.plan["roles"]["candidate"] = copy.deepcopy(self.plan["roles"]["baseline"])
        plan_path = self.root / "selected-plan.json"
        collector.save(plan_path, self.plan)
        review = {"metrics": {OP: trial_contract(hard_ceiling_us=999)}}
        with mock.patch.object(collector, "admission", return_value=(review, [])), \
                mock.patch.object(collector, "run_process", return_value=record_for(self.output)) as launch:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                code = collector.main(["calibrate", "--plan", str(plan_path), "--output", str(self.root / "calibration")])
        self.assertEqual((code, launch.call_count), (1, 1))
        result = collector.load(self.root / "calibration" / "summary.json")
        self.assertIn("numerical", {problem["category"] for problem in result["issues"]})
        self.assertEqual(result["observation_issues"], [])
        self.assertEqual(len(result["unrun"]), 5)
        self.assertEqual(result["qualification_status"], "failed")

    def test_smoke_numeric_findings_never_hide_semantic_or_cleanup_failure(self):
        review = {"metrics": {OP: trial_contract(hard_ceiling_us=999)}}
        self.output["oracle"]["passed"] = False
        record = record_for(self.output)
        record["cleanup_ok"] = False
        with mock.patch.object(collector, "admission", return_value=(review, [])), \
                mock.patch.object(collector, "run_process", return_value=record), \
                contextlib.redirect_stdout(io.StringIO()):
            result = collector.collect(self.plan, "smoke", self.root / "unsafe")
        self.assertEqual(result["execution_status"], "failed")
        self.assertTrue({"cleanup", "semantics"} <= {problem["category"] for problem in result["issues"]})
        with mock.patch.object(collector, "run_process") as launch, contextlib.redirect_stdout(io.StringIO()):
            refused = collector.collect(self.plan, "smoke", self.root / "new-output")
        launch.assert_not_called()
        self.assertTrue(refused["issues"])

    def test_historical_smoke_numeric_failure_is_not_regraded_as_current_observation(self):
        directory = (self.root / "historical").resolve()
        directory.mkdir()
        review = {"metrics": {OP: trial_contract(hard_ceiling_us=999)}}
        plan_binding = collector.save(directory / "plan.json", self.plan)
        slot = collector.schedule("smoke")[0]
        record = record_for(self.output)
        validation = {"slot": slot, **collector.reconcile(self.plan, slot, record, review)}
        attempt = {"slot": slot}
        for kind, payload in (
                ("started", {"slot": slot, "plan_sha256": plan_binding["sha256"]}),
                ("raw", {"slot": slot, "plan_sha256": plan_binding["sha256"], "record": record,
                         "execution_limits": {"timeout_seconds": self.plan["bounds"]["primary_seconds"],
                                              "cleanup_seconds": self.plan["bounds"]["cleanup_seconds"],
                                              "output_limit_bytes": self.plan["bounds"]["output_limit_bytes"]}}),
                ("validation", validation)):
            name = slot["id"] + "." + kind + ".json"
            binding = collector.save(directory / name, payload)
            attempt[kind] = {"name": name, "sha256": binding["sha256"]}
        registration = self.campaign.root / "phases" / (self.plan["case"] + "-smoke.json")
        collector.save(registration, {"output": str(directory), "stage": "smoke",
                                      "plan_sha256": plan_binding["sha256"]})
        summary = {"format": "consumer-phase-v1", "stage": "smoke", "case": self.plan["case"],
                   "plan_sha256": plan_binding["sha256"], "registration": collector.reference(registration),
                   "schedule": [slot], "attempts": [attempt], "unrun": [], "issues": validation["issues"],
                   "admission_issues": [], "qualification": False}
        seal = collector.save(directory / "summary.json", summary)
        collector.save(registration.with_suffix(".complete"), seal)
        before = {path: path.read_bytes() for path in directory.iterdir()}
        with mock.patch.object(collector, "admission", return_value=(review, [])), \
                mock.patch.object(collector, "run_process") as launch, \
                contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(collector.main(["report", str(directory)]), 1)
        launch.assert_not_called()
        result = collector.strict_json(output.getvalue())
        self.assertIn("numerical", {problem["category"] for problem in result["issues"]})
        self.assertNotIn("observation_issues", result)
        self.assertEqual(before, {path: path.read_bytes() for path in before})
        with mock.patch.object(collector, "reconcile", side_effect=AssertionError("historical data is not regraded")), \
                mock.patch.object(collector, "admission", side_effect=AssertionError("historical review is not upgraded")):
            retained = collector.load_cohort(directory)
        self.assertEqual(retained["summary"], summary)
        self.assertEqual(retained["rows"], [validation])
        (directory / "00.validation.json").write_text("{}\n")
        with self.assertRaises(ValueError):
            collector.load_cohort(directory)

    def test_windows_smoke_keeps_missing_optional_runtime_and_cpu_accounting(self):
        self.plan["target"] = collector.component.TARGETS[1]
        self.plan["host"]["target"] = self.plan["target"]
        record = record_for(trial_output(self.plan))
        record["cpu"].update(status="unknown", final=False, reason="job accounting unavailable",
                             user_seconds=None, system_seconds=None,
                             source="windows-job-basic-accounting", scope="owned-job-including-nested-jobs",
                             source_unit_seconds=0.0000001)
        # A distinct host has its own campaign; no foreign target/host fence is bypassed.
        self.plan["campaign"] = str(self.root / "windows-campaign")
        with mock.patch.object(collector, "run_process", return_value=record) as launch, \
                contextlib.redirect_stdout(io.StringIO()):
            result = collector.collect(self.plan, "smoke", self.root / "windows-smoke")
        self.assertEqual((launch.call_count, result["issues"]), (1, []))
        self.assertFalse(result["qualification"])
        validation = collector.load(self.root / "windows-smoke/00.validation.json")
        self.assertIsNone(validation["metrics"]["lifetime_cpu_seconds"])
        self.assertIsNone(validation["raw_cpu_observation"]["user_plus_system_seconds"])
        self.assertEqual(validation["cpu"], record["cpu"])
        self.assertTrue(validation["unavailable"])


class DiscoveryFailures(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.plan = plan_for(self.root)
        self.plan["target"] = collector.runtime.APPLE
        self.plan["host"]["target"] = collector.runtime.APPLE
        self.plan["loader"] = True
        self.plan["env"]["DYLD_PRINT_LIBRARIES"] = "1"
        self.enterContext(mock.patch.object(collector, "host_identity", return_value=self.plan["host"]))
        self.enterContext(mock.patch.object(collector.runtime, "system_snapshot", return_value={}))

    def test_raw_persistence_failure_preserves_cleanup_fence_for_other_case(self):
        durable_save = collector.save
        first = self.root / "first"

        def fail_raw(path, value, maximum=4 * collector.MIB):
            if Path(path) == first / "raw.json":
                raise OSError("injected raw persistence failure")
            return durable_save(path, value, maximum)

        with mock.patch.object(collector, "save", side_effect=fail_raw), \
                mock.patch.object(collector, "run_process", return_value={"cleanup_ok": False}) as launch:
            result = collector.discover_runtime(self.plan, "baseline", first)
            launch.assert_called_once()
        self.assertIsNone(result["raw"])
        self.assertIn("cleanup", {row["category"] for row in result["issues"]})
        self.assertIn("cleanup", {row["category"] for row in collector.Campaign(self.plan).fences()})
        later = copy.deepcopy(self.plan)
        later["case"] = "trial-miss-roi"
        later["roles"]["baseline"]["argv"][6] = "miss-roi"
        with mock.patch.object(collector, "run_process") as launch:
            refused = collector.discover_runtime(later, "baseline", self.root / "later")
        launch.assert_not_called()
        self.assertIn("cleanup", {row["category"] for row in refused["issues"]})

    def test_durable_start_cannot_launch_after_discovery_reservation_expires(self):
        clock = [100.0]
        durable_save = collector.save

        def delayed_start(path, value, maximum=4 * collector.MIB):
            binding = durable_save(path, value, maximum)
            if Path(path).name == "started.json":
                clock[0] += 226
            return binding

        with mock.patch.object(collector.time, "monotonic", side_effect=lambda: clock[0]), \
                mock.patch.object(collector, "save", side_effect=delayed_start), \
                mock.patch.object(collector, "run_process", return_value={"cleanup_ok": True, "stdout": "{}"}) as launch:
            result = collector.discover_runtime(self.plan, "baseline", self.root / "late")
        launch.assert_not_called()
        self.assertEqual(result["launch_disposition"], "not_launched")
        self.assertIsNone(result["raw"])
        self.assertIn("deadline", {row["category"] for row in result["issues"]})
        self.assertNotIn("cleanup", {row["category"] for row in result["issues"]})
        self.assertEqual(collector.Campaign(self.plan).fences(), [])


@unittest.skipIf(collector.os.name == "nt", "Apple dyld fixtures require absolute POSIX image paths")
class AppleSourceClosedEvidence(unittest.TestCase):
    """Real file/UUID/parser/receipt checks; only host cache APIs are substituted."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.plan = plan_for(self.root)
        self.plan.update(target=collector.runtime.APPLE, loader=True)
        self.plan["env"]["DYLD_PRINT_LIBRARIES"] = "1"
        self.engine = self.root / "engine"
        self.driver = self.root / "driver"
        self.engine.write_bytes(binary_fixtures.macho(binary_fixtures.UUID_B, filetype=2))
        self.driver.write_bytes(binary_fixtures.macho(binary_fixtures.UUID_A, filetype=2))
        (self.root / "environment.json").write_text('{"provider":"cpu"}\n')
        self.source = self.root / "selected.rs"
        self.source.write_text("fn selected() { spawn(); }\nfn terminal() { wait(); join(); }\n// outside selected sections\n")
        self.plan["inputs"][str(self.source)] = {}
        for path in self.plan["inputs"]:
            self.plan["inputs"][path] = {"sha256": collector.sha256(Path(path))}
        for binary in self.plan["roles"]["baseline"].values():
            if isinstance(binary, dict):
                binary["sha256"] = self.plan["inputs"][binary["path"]]["sha256"]
        self.output = trial_output(self.plan)
        self.sinks = self.root / "sinks"
        self.sinks.mkdir()
        view = self.output["operation"]["controller"]
        for child in (view["attempts"][0], view["result"]):
            pid = child["child_pid"]
            text = (binary_fixtures.load_line(pid, binary_fixtures.UUID_B, self.engine)
                    + binary_fixtures.load_line(pid, binary_fixtures.UUID_C, binary_fixtures.SYSTEM_IMAGE))
            path = self.sinks / f"{pid}.stderr"
            path.write_text(text)
            child["stderr_observation"] = {
                "loader_enabled": True, "child_pid": pid, "complete": True, "eof": True,
                "method": collector.runtime.images.METHOD, "zero_loader_lines": False,
                "bytes": len(text.encode()), "bytes_total": len(text.encode()),
                "application_bytes_total": 0, "application_bytes_retained": 0,
                "read_error": None, "read_limit_reached": False, "truncated": False, "unterminated_tail": False,
                "foreign_lines": 0, "interleaved": 0, "malformed_prefixes": 0, "oversized_lines": 0,
                "foreign_pid_entries_dropped": 0, "foreign_pids": [], "lines": 2,
                "sink": {"path": str(path), "bytes_written": len(text.encode()), "limit": 8 * collector.MIB,
                         "truncated": False, "write_error": None}}
        self.record = record_for(self.output)
        self.record["stderr"] = (binary_fixtures.load_line(100, binary_fixtures.UUID_A, self.driver)
                                 + binary_fixtures.load_line(100, binary_fixtures.UUID_C, binary_fixtures.SYSTEM_IMAGE))
        for name, value in (("_system_identity", binary_fixtures.SYSTEM), ("_dyld_identity", binary_fixtures.DYLD)):
            patcher = mock.patch.object(collector.runtime.images, name, return_value=copy.deepcopy(value))
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = mock.patch.object(collector.runtime.images, "_cache_contains_path",
                                   side_effect=lambda path: path == binary_fixtures.SYSTEM_IMAGE)
        patcher.start()
        self.addCleanup(patcher.stop)
        before = collector.runtime.system_snapshot()
        data = collector.runtime.discover(self.plan, "baseline", self.record, self.output, self.sinks,
                                          before, collector.time.monotonic() + 60)
        raw = {"role": "baseline", "execution_binding": collector.runtime.execution_binding(self.plan),
               "execution_limits": {"timeout_seconds": self.plan["bounds"]["primary_seconds"],
                                    "cleanup_seconds": self.plan["bounds"]["cleanup_seconds"],
                                    "output_limit_bytes": self.plan["bounds"]["output_limit_bytes"]},
               "record": self.record}
        data.update(raw=collector.save(self.root / "discovery-raw.json", raw), sink_root=str(self.sinks), issues=[])
        origin = collector.save(self.root / "discovery.json", data)
        self.bundle = collector.runtime.closure_from(self.plan, [data])
        self.bundle["origins"] = [origin]
        self.bundle["freeze"] = {"baseline": collector.runtime.snapshot(self.plan, self.bundle, "baseline", collector.time.monotonic() + 60)}
        self.plan["runtime_closure"] = collector.save(self.root / "closure.json", self.bundle)
        self.context = collector.context_digest(self.plan)
        source = {"path": str(self.source), "sha256": collector.sha256(self.source), "sections": [{
            "start_line": 1, "end_line": 2,
            "sha256": collector.hashlib.sha256(b"fn selected() { spawn(); }\nfn terminal() { wait(); join(); }\n").hexdigest(),
            "purpose": "selected creation and awaited terminal path"}]}
        self.sources = [source]
        self.native = {"kind": "native-process-closure-evidence-v1", "case": self.plan["case"],
                       "context_sha256": self.context, "claim": "no-unwitnessed-owned-descendants",
                       "reviewer": "fixture-reviewer", "rationale": "selected fixture path only", "residuals": [],
                       "artifacts": [{"path": row["path"], "sha256": row["sha256"]}
                                     for row in collector.runtime.all_rows(self.bundle, "baseline") if row["class"] == "file"],
                       "source_files": self.sources}
        native_ref = collector.save(self.root / "native-proof.json", self.native)
        self.certificate = {
            "kind": "source-owned-processes-v1", "accepted": True, "target": self.plan["target"],
            "context_sha256": self.context, "reviewer": "fixture-reviewer", "rationale": "selected fixture path only",
            "scope": {"owned_tree_claim": "all descendants of the selected application root",
                      "excluded": ["out-of-process system services", "manual injection"]},
            "source_files": self.sources,
            "roles": {"baseline": {
                "case": self.plan["case"], "application_process_cap": 3, "witness_contract": collector.runtime.WITNESS_TRIAL,
                "executables": {name: {key: value[key] for key in ("path", "sha256")}
                                for name, value in collector.runtime.executables(self.plan, "baseline").items()},
                "spawn_sites": [{"source_path": str(self.source), "line": 1, "parent_role": "root", "child_role": name, "maximum_calls": 1}
                                for name in ("capability", "recognition")],
                "terminal_source_files": [str(self.source)], "native_dependency_evidence": [native_ref]}}}
        certificate_ref = collector.save(self.root / "certificate.json", self.certificate)
        self.review = {"kind": collector.METHOD_REVIEW, "accepted": True, "context_sha256": self.context,
                       "reviewer": "fixture-reviewer", "rationale": "deterministic contract fixture",
                       "owned_processes": certificate_ref,
                       "metrics": {"operation_us": trial_contract()},
                       "gates": {}}
        for gate in collector.GATES:
            evidence = {"gate": gate, "complete": True, "context_sha256": self.context,
                        "source_files": {str(self.source): collector.sha256(self.source)},
                        "closure": self.plan["runtime_closure"], "source_certificate": certificate_ref}
            if gate == "resources":
                evidence["trial_responsiveness"] = trial_declaration()
            self.review["gates"][gate] = {"status": "accepted", "issues": [],
                                          "evidence": [collector.save(self.root / (gate + ".json"), evidence)]}
        self.plan["admission"] = collector.save(self.root / "review.json", self.review)
        self.observed = {"closure_sha256": collector.runtime.images.canonical_sha256(self.bundle),
                         "pre": self.bundle["freeze"]["baseline"],
                         "post": collector.runtime.snapshot(self.plan, self.bundle, "baseline", collector.time.monotonic() + 60),
                         "issues": []}

    def reconcile(self, output=None, record=None, observation=None):
        current = copy.deepcopy(self.record if record is None else record)
        if output is not None:
            current["stdout"] = collector.document(output).decode()
        return collector.reconcile(self.plan, collector.schedule("smoke")[0], current, self.review,
                                   self.sinks, self.observed if observation is None else observation)

    def certificate_check(self, value):
        return collector.runtime.validate_process_certificate(self.plan, value, self.bundle, self.context, collector.read_reference)

    def test_actual_role_closure_and_ownership_do_not_rewrite_raw_cpu(self):
        self.assertEqual(collector.admission(self.plan)[1], [])
        before = (self.root / "discovery-raw.json").read_bytes()
        result = self.reconcile()
        self.assertEqual(result["issues"], [])
        self.assertEqual(result["unavailable"], [])
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])
        self.assertEqual(result["raw_cpu_observation"]["user_plus_system_seconds"], "3/25")
        self.assertEqual(result["owned_processes"]["roles"], {"root": 100, "capability": 101, "recognition": 102})
        self.assertEqual(result["cpu"], self.record["cpu"])
        self.assertEqual(result["cpu"]["status"], "partial")
        self.assertIsNone(result["cpu_resolution"])
        self.assertTrue(result["cpu_coverage"]["issues"])
        self.assertEqual((self.root / "discovery-raw.json").read_bytes(), before)

    def test_bound_resident_policy_completes_admission_and_allows_actual_point(self):
        resources = collector.read_reference(self.review["gates"]["resources"]["evidence"][0])
        review = copy.deepcopy(self.review)
        review["metrics"][RSS] = resident_contract()
        review["gates"]["resources"]["evidence"] = [collector.save(
            self.root / "resources-resident.json", {**resources, "resident_policies": resident_policy()})]
        self.plan["admission"] = collector.save(self.root / "review-resident.json", review)
        self.assertEqual(collector.admission(self.plan)[1], [])
        self.review = review
        result = self.reconcile()
        self.assertEqual((result["issues"], result["unavailable"]), ([], []))
        self.assertEqual(result["metrics"][RSS], 100 * collector.MIB)
        self.assertTrue(result["ungraded_resource_domains"]["os_high_water"].startswith("unavailable"))

    def test_bare_accepted_certificate_and_unproved_native_paths_refuse(self):
        with self.assertRaises((KeyError, ValueError)):
            self.certificate_check({"kind": "source-owned-processes-v1", "accepted": True})
        bad = copy.deepcopy(self.certificate)
        bad["roles"]["baseline"]["native_dependency_evidence"] = []
        with self.assertRaisesRegex(ValueError, "native/producer"):
            self.certificate_check(bad)
        proof = {**self.native, "residuals": ["unproved reachable child path"]}
        bad["roles"]["baseline"]["native_dependency_evidence"] = [collector.save(self.root / "native-unclosed.json", proof)]
        with self.assertRaisesRegex(ValueError, "case/source"):
            self.certificate_check(bad)

    def test_current_review_cannot_select_cpu_for_grading(self):
        self.assertEqual(collector.admission(self.plan)[1], [])
        review = copy.deepcopy(self.review)
        review["metrics"]["lifetime_cpu_seconds"] = {
            "unit": "s", "api": "posix-wait4", "owner": "owned tree", "interval": "whole invocation",
            "missing_reason": None, "quantum": "1/1000000", "resolution": "1/1000000",
            "resolution_method": "fixture"}
        self.plan["admission"] = collector.save(self.root / "review-with-cpu.json", review)
        _, failures = collector.admission(self.plan)
        self.assertTrue(failures)

    def test_non_cpu_admission_guards_remain_mandatory(self):
        for gate in ("source", "runtime_images", "owned_processes", "clocks", "resources", "retention"):
            with self.subTest(gate=gate):
                review = copy.deepcopy(self.review)
                review["gates"].pop(gate)
                self.plan["admission"] = collector.save(self.root / (gate + "-missing-review.json"), review)
                self.assertTrue(collector.admission(self.plan)[1])
        review = copy.deepcopy(self.review)
        review.pop("owned_processes")
        self.plan["admission"] = collector.save(self.root / "ownership-missing-review.json", review)
        self.assertTrue(collector.admission(self.plan)[1])
        windows = {**self.plan, "target": collector.component.TARGETS[1]}
        with self.assertRaises(ValueError):
            collector.runtime.validate_bundle(windows, self.bundle, collector.read_reference, collector.semantic_output)

    def test_whole_origin_file_change_outside_selected_snippet_invalidates_certificate(self):
        self.source.write_text("fn selected() { spawn(); }\nfn terminal() { wait(); join(); }\nfn extra() { spawn(); }\n")
        with self.assertRaisesRegex(ValueError, "originating file changed"):
            self.certificate_check(self.certificate)
        result = self.reconcile()
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])
        self.assertNotIn("owned_processes", result)

    def test_constant_reaped_flag_cannot_replace_actual_wait_return_witness(self):
        output = copy.deepcopy(self.output)
        output["operation"]["controller"]["result"]["supervision_observation"]["root_wait_returned_ns"] = None
        result = self.reconcile(output)
        self.assertTrue(result["issues"])
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])
        self.assertEqual(result["cpu"]["status"], "partial")

    def test_foreign_root_pid_and_new_loaded_image_fail_identity(self):
        foreign = {**self.record, "process_id": 999}
        self.assertIn("identity", {row["category"] for row in self.reconcile(record=foreign)["issues"]})
        added = {**self.record, "stderr": self.record["stderr"] + binary_fixtures.load_line(100, binary_fixtures.UUID_D, "/unknown/runtime.dylib")}
        result = self.reconcile(record=added)
        self.assertTrue(any(row["category"] == "identity" and "unknown loaded image" in row["reason"] for row in result["issues"]))
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])

    def test_incomplete_child_sink_cannot_become_image_or_cpu_coverage(self):
        output = copy.deepcopy(self.output)
        output["operation"]["controller"]["result"]["stderr_observation"]["complete"] = False
        result = self.reconcile(output)
        self.assertTrue(result["issues"])
        self.assertNotIn("runtime_images", result)
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])

    def test_missing_transport_counter_is_not_assumed_zero(self):
        output = copy.deepcopy(self.output)
        output["operation"]["controller"]["result"]["stderr_observation"].pop("foreign_pid_entries_dropped")
        result = self.reconcile(output)
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])
        self.assertNotIn("owned_processes", result)

    def test_discovery_enforcement_and_observed_roles_cannot_be_relabelled(self):
        discovery = collector.load(self.root / "discovery.json")
        raw = collector.read_reference(discovery["raw"])
        raw["execution_limits"]["timeout_seconds"] = 60
        with self.assertRaisesRegex(ValueError, "enforcing envelope"):
            collector.runtime.validate_discovery_raw(self.plan, discovery, raw)
        plan = copy.deepcopy(self.plan)
        plan["roles"]["candidate"] = copy.deepcopy(plan["roles"]["baseline"])
        with self.assertRaisesRegex(ValueError, "actual discovery"):
            collector.runtime.closure_from(plan, [discovery])

    def test_expired_post_binding_is_missing_evidence_not_identity_drift(self):
        post = collector.capture_runtime(self.plan, self.bundle, "baseline", collector.time.monotonic() - 1)
        self.assertIsNone(post["value"])
        self.assertEqual({row["category"] for row in post["issues"]}, {"unavailable"})
        observed = {**self.observed, "post": None, "issues": post["issues"]}
        result = self.reconcile(observation=observed)
        self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])
        self.assertNotIn("identity", {row["category"] for row in result["issues"]})

    def test_verified_cache_membership_and_post_file_identity_are_required(self):
        with mock.patch.object(collector.runtime.images, "_cache_contains_path", return_value=False):
            result = collector.capture_runtime(self.plan, self.bundle, "baseline", collector.time.monotonic() + 60)
        self.assertEqual(result["issues"][0]["category"], "identity")
        self.engine.write_bytes(self.engine.read_bytes() + b"changed")
        result = collector.capture_runtime(self.plan, self.bundle, "baseline", collector.time.monotonic() + 60)
        self.assertEqual(result["issues"][0]["category"], "identity")
        self.assertIsNone(result["value"])

    def test_current_ownership_needs_no_cpu_rollup_or_resolution_certificate(self):
        self.assertNotIn("source_closed_cpu", self.review)
        self.assertNotIn("accounting_allowance", self.review)
        self.assertNotIn("kernel_rollup_evidence", self.certificate)
        self.assertEqual(collector.admission(self.plan)[1], [])
        variants = [
            {**self.record["cpu"], "user_seconds": 10**12, "system_seconds": 10**10},
            {**self.record["cpu"], "status": "unknown", "final": False,
             "user_seconds": None, "system_seconds": None, "reason": "wait4_unavailable"},
            {**self.record["cpu"], "status": "complete", "reason": None,
             "source": "windows-job-basic-accounting", "scope": "owned-job-including-nested-jobs",
             "source_unit_seconds": 0.0000001},
        ]
        for cpu in variants:
            with self.subTest(cpu=cpu):
                result = self.reconcile(record={**self.record, "cpu": cpu})
                self.assertEqual((result["issues"], result["unavailable"]), ([], []))
                self.assertEqual(result["cpu"], cpu)
                self.assertEqual(result["cpu_coverage"]["status"], cpu["status"])
                self.assertEqual(result["owned_processes"]["roles"],
                                 {"root": 100, "capability": 101, "recognition": 102})
                if cpu["status"] != "complete":
                    self.assertIsNone(result["metrics"]["lifetime_cpu_seconds"])

    def test_unknown_cpu_does_not_mask_semantic_cleanup_or_runtime_barriers(self):
        record = copy.deepcopy(self.record)
        record["cpu"].update(status="unknown", final=False, reason="wait4_unavailable",
                             user_seconds=None, system_seconds=None)
        bad = copy.deepcopy(self.output)
        bad["oracle"]["passed"] = False
        result = self.reconcile(output=bad, record=record)
        self.assertIn("semantics", {problem["category"] for problem in result["issues"]})
        result = self.reconcile(record={**record, "cleanup_ok": False})
        self.assertIn("cleanup", {problem["category"] for problem in result["issues"]})
        result = self.reconcile(record={**record, "process_id": 999})
        self.assertIn("identity", {problem["category"] for problem in result["issues"]})
        result = self.reconcile(record=record, observation={**self.observed, "post": None})
        self.assertTrue(result["unavailable"])

    def test_malformed_cpu_still_fails_with_all_independent_method_evidence(self):
        record = copy.deepcopy(self.record)
        record["cpu"]["user_seconds"] = -1
        result = self.reconcile(record=record)
        self.assertIn("semantics", {problem["category"] for problem in result["issues"]})
        self.assertEqual(result["metrics"][OP], 1000)

    def test_native_environment_override_is_not_waived_by_certificate_acceptance(self):
        self.plan["env"]["ORT_ADDR2LINE"] = "1"
        context = collector.context_digest(self.plan)
        certificate = {**self.certificate, "context_sha256": context}
        with self.assertRaisesRegex(ValueError, "environment overrides"):
            collector.runtime.validate_process_certificate(self.plan, certificate, self.bundle, context, collector.read_reference)

    def test_noisy_and_unavailable_cpu_leave_current_latency_and_resource_grades_unchanged(self):
        review = {**self.review, "metrics": {OP: trial_contract(), RSS: resident_contract()}}

        def rows(stage):
            result = []
            for index, slot in enumerate(collector.schedule(stage)):
                cpu = {**self.record["cpu"], "user_seconds": 10 ** (index + 1),
                       "system_seconds": index, "accounting_resolution_seconds": index + 1}
                if index % 3 == 0:
                    cpu.update(status="unknown", final=False, reason="wait4_unavailable",
                               user_seconds=None, system_seconds=None)
                observation = self.reconcile(record={**self.record, "cpu": cpu})
                self.assertEqual((observation["issues"], observation["unavailable"]), ([], []))
                result.append({"slot": slot, **observation})
            return result

        calibration = rows("calibration")
        envelopes = collector.derive_envelopes(calibration, review, self.plan["case"])
        self.assertEqual(set(envelopes), {OP, RSS})
        for stage in ("control", "comparison"):
            observations = rows(stage)
            self.assertEqual(collector.grade_rows(observations, stage, envelopes, self.plan["case"]), [])
            observations[0]["metrics"][OP] = TRIAL_CEILING + 1
            self.assertIn("numerical", {problem["category"] for problem in
                                       collector.grade_rows(observations, stage, envelopes, self.plan["case"])})
            observations[0]["metrics"][OP] = 1000
            observations[0]["metrics"][RSS] = HARD + 1
            self.assertIn("numerical", {problem["category"] for problem in
                                       collector.grade_rows(observations, stage, envelopes, self.plan["case"])})


class ProfileAcceptance(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.plan = plan_for(self.root)
        collector.Campaign(self.plan).initialize()
        review = {"kind": collector.METHOD_REVIEW, "metrics": {OP: trial_contract()}}
        self.calibration = {"plan": copy.deepcopy(self.plan), "summary": {"format": collector.PHASE_FORMAT, "stage": "calibration", "attempts": []},
                            "rows": cohort_rows("calibration"), "review": review, "issues": [],
                            "directory": str(self.root / "calibration"), "binding": {"path": str(self.root / "calibration-summary"), "sha256": "a" * 64}}
        control_plan = copy.deepcopy(self.plan)
        control_plan["calibration"] = {key: self.calibration[key] for key in ("directory", "binding")}
        self.control = {"plan": control_plan, "summary": {"format": collector.PHASE_FORMAT, "stage": "control", "attempts": []},
                        "rows": cohort_rows("control"), "review": review, "issues": [],
                        "directory": str(self.root / "control"), "binding": {"path": str(self.root / "control-summary"), "sha256": "b" * 64}}
        self.proposal = collector.profile_proposal(self.calibration, self.control)
        self.review = {"accepted": True, "profile_proposal_sha256": collector.hashed(self.proposal),
                       "reviewer": "fixture-reviewer", "rationale": "exact independent six/twelve proposal"}
        self.profile = {**copy.deepcopy(self.proposal), "proposal_sha256": collector.hashed(self.proposal),
                        "acceptance": collector.save(self.root / "acceptance.json", self.review)}

    def test_acceptance_hash_binds_all_proposal_fields_not_just_stored_digest(self):
        self.assertEqual(collector.accepted_profile(self.proposal, self.profile, self.review), self.proposal["envelopes"])
        for key, value in (("case", "trial-miss-roi"), ("calibration", self.profile["control"]),
                           ("proposal_sha256", "0" * 64), ("context_sha256", "0" * 64)):
            with self.subTest(field=key), self.assertRaises(ValueError):
                collector.accepted_profile(self.proposal, {**self.profile, key: value}, self.review)
        for field in ("reviewer", "rationale"):
            with self.subTest(field=field), self.assertRaises(ValueError):
                collector.accepted_profile(self.proposal, self.profile, {**self.review, field: ""})

    def test_control_lineage_and_origin_campaign_fences_survive_passing_numbers(self):
        self.control["plan"]["calibration"]["binding"] = {"path": "/different", "sha256": "c" * 64}
        with self.assertRaisesRegex(ValueError, "calibration binding"):
            collector.profile_proposal(self.calibration, self.control)
        self.control["plan"]["calibration"] = {key: self.calibration[key] for key in ("directory", "binding")}
        collector.Campaign(self.plan).fence("identity", "source changed", "prior-case")
        with self.assertRaisesRegex(ValueError, "campaign"):
            collector.profile_proposal(self.calibration, self.control)

    def test_unchanged_resident_control_is_graded_before_profile_freeze(self):
        self.calibration["review"]["metrics"][RSS] = resident_contract()
        for row in self.calibration["rows"] + self.control["rows"]:
            row["metrics"][RSS] = 100 * collector.MIB
        envelope = collector.profile_proposal(self.calibration, self.control)["envelopes"][RSS]
        self.assertEqual((envelope["ceiling"], envelope["allowance"]), (str(HARD), str(FLOOR)))
        for row in self.control["rows"]:
            if row["slot"]["role"] == "control":
                row["metrics"][RSS] += FLOOR + 1
        with self.assertRaisesRegex(ValueError, "unchanged control failed"):
            collector.profile_proposal(self.calibration, self.control)

    def test_trial_budget_is_bound_through_full_profile_recomputation(self):
        envelope = self.proposal["envelopes"][OP]
        self.assertEqual({key: envelope[key] for key in collector.TRIAL_FIELDS}, trial_budget())
        self.assertEqual((envelope["ceiling"], envelope["allowance"]), (str(TRIAL_CEILING), str(TRIAL_REGRESSION)))
        for key, value in (("policy", "fresh-trial-responsiveness-v0"), ("population", "warm-fresh-process"),
                           ("hard_ceiling_us", TRIAL_CEILING + 1), ("max_regression_us", TRIAL_REGRESSION + 1),
                           ("ceiling", str(TRIAL_CEILING + 1)), ("allowance", str(TRIAL_REGRESSION + 1)),
                           ("central_four", {"spread": "0", "mean": "1000", "input_indices": [0, 1, 2, 3]})):
            tampered = copy.deepcopy(self.profile)
            tampered["envelopes"][OP][key] = value
            with self.subTest(field=key), self.assertRaisesRegex(ValueError, "contents changed"):
                collector.accepted_profile(self.proposal, tampered, self.review)
        # A changed declared budget recomputes a proposal that the earlier acceptance does not bind.
        for changes in ({"hard_ceiling_us": TRIAL_CEILING + 1}, {"max_regression_us": TRIAL_REGRESSION + 1}):
            self.calibration["review"]["metrics"] = {OP: trial_contract(**changes)}
            changed = collector.profile_proposal(self.calibration, self.control)
            with self.subTest(changes=changes), self.assertRaisesRegex(ValueError, "contents changed"):
                collector.accepted_profile(changed, self.profile, self.review)
        # An earlier scalar trial contract carries no policy and is never reinterpreted.
        self.calibration["review"]["metrics"] = {OP: {"unit": "us", "quantum": 1, "resolution": 1}}
        with self.assertRaisesRegex(ValueError, "policy/population"):
            collector.profile_proposal(self.calibration, self.control)

    def test_inconclusive_trial_calibration_or_control_blocks_profile(self):
        for row in self.control["rows"]:
            if row["slot"]["role"] == "control":
                row["metrics"][OP] += TRIAL_REGRESSION
        collector.profile_proposal(self.calibration, self.control)
        for row in self.control["rows"]:
            if row["slot"]["role"] == "control":
                row["metrics"][OP] += 1
        with self.assertRaisesRegex(ValueError, "unchanged control failed"):
            collector.profile_proposal(self.calibration, self.control)
        self.control["rows"] = cohort_rows("control")
        for row in self.calibration["rows"][:2]:
            row["metrics"][OP] += TRIAL_REGRESSION + 1
        with self.assertRaises(collector.ContractError) as caught:
            collector.profile_proposal(self.calibration, self.control)
        self.assertEqual(caught.exception.category, "inconclusive")

    def test_profile_keeps_noisy_and_missing_cpu_diagnostic_only(self):
        for cohort in (self.calibration, self.control):
            for index, row in enumerate(cohort["rows"]):
                row["metrics"]["lifetime_cpu_seconds"] = None if index % 2 else str(10 ** index)
                row["cpu_resolution"] = None if index % 2 else str(10 ** index)
        proposal = collector.profile_proposal(self.calibration, self.control)
        self.assertEqual(proposal, self.proposal)
        self.assertEqual(collector.accepted_profile(proposal, self.profile, self.review),
                         {OP: self.proposal["envelopes"][OP]})

    def test_historical_cohorts_and_method_reviews_cannot_freeze_current_profile(self):
        for cohort in (self.calibration, self.control):
            for format in ("consumer-phase-v1", "consumer-phase-v2"):
                current = cohort["summary"]["format"]
                cohort["summary"]["format"] = format
                with self.subTest(format=format), self.assertRaises(ValueError):
                    collector.profile_proposal(self.calibration, self.control)
                cohort["summary"]["format"] = current
        self.calibration["review"]["kind"] = "consumer-method-review-v4"
        with self.assertRaises(ValueError):
            collector.profile_proposal(self.calibration, self.control)


class BoundReview(unittest.TestCase):
    """Real hash-bound review/evidence files; unrelated runtime/ownership gates stay unavailable here."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.plan = plan_for(self.root)
        self.context = collector.context_digest(self.plan)
        self.source = str(self.root / "driver")
        self.files = 0

    def bound(self, value):
        self.files += 1
        return collector.save(self.root / f"bound-{self.files}.json", value)

    def evidence(self, gate, **fields):
        return self.bound({"gate": gate, "complete": True, "context_sha256": self.context,
                           "source_files": {self.source: self.plan["inputs"][self.source]["sha256"]}, **fields})

    def review_issues(self, metrics, *resources, kind=collector.METHOD_REVIEW, prefixes=()):
        """Admission failures of the method record, resources gate and the named metrics only."""
        gates = {gate: {"status": "accepted", "issues": [], "evidence": [self.evidence(gate)]}
                 for gate in collector.GATES}
        gates["resources"]["evidence"] = [self.evidence("resources", **fields) for fields in resources or ({},)]
        self.plan["admission"] = self.bound({
            "kind": kind, "accepted": True, "context_sha256": self.context, "reviewer": "fixture-reviewer",
            "rationale": "bound policy fixture", "gates": gates, "metrics": metrics})
        return [row for row in collector.admission(self.plan)[1]
                if row["category"] == "admission" or row["reason"].startswith(("resources:", *prefixes))]


class ResidentAdmission(BoundReview):
    def resident_issues(self, metrics, *resources, kind=collector.METHOD_REVIEW):
        return self.review_issues(metrics, *resources, kind=kind, prefixes=(RSS,))

    def test_matching_bound_policy_admits_parameterized_resident_contract(self):
        policy = {"resident_policies": resident_policy()}
        self.assertEqual(self.resident_issues({RSS: resident_contract()}, policy), [])
        # A bound resources record without a policy neither satisfies nor contradicts another.
        self.assertEqual(self.resident_issues({RSS: resident_contract()}, {}, policy), [])
        other = {"hard_ceiling_bytes": 3 * HARD, "comparison_allowance_floor_bytes": 1}
        self.assertEqual(self.resident_issues({RSS: resident_contract(**other)},
                                              {"resident_policies": resident_policy(**other)}), [])

    def test_missing_mismatched_unbound_or_inconsistent_policy_refuses(self):
        policy = {"resident_policies": resident_policy()}
        cases = {
            "missing": [{}],
            "mismatched ceiling": [{"resident_policies": resident_policy(hard_ceiling_bytes=HARD - 1)}],
            "mismatched floor": [{"resident_policies": resident_policy(comparison_allowance_floor_bytes=FLOOR + 1)}],
            "inconsistent second record": [policy, {"resident_policies": resident_policy(hard_ceiling_bytes=HARD + 1)}],
            "other context": [{**policy, "context_sha256": "0" * 64}],
            "unbound source": [{**policy, "source_files": {self.source: "0" * 64}}],
            "empty justification": [{"resident_policies": resident_policy(justification=" ")}],
            "unselected metric": [{"resident_policies": {**resident_policy(),
                                                         "child_sampled_rss_bytes": resident_policy()[RSS]}}],
            "not an object": [{"resident_policies": [resident_policy()[RSS]]}],
        }
        for name, resources in cases.items():
            with self.subTest(case=name):
                self.assertTrue(self.resident_issues({RSS: resident_contract()}, *resources))

    def test_invalid_numeric_fields_refuse_even_when_policy_repeats_them(self):
        for key in ("hard_ceiling_bytes", "comparison_allowance_floor_bytes"):
            for value in (None, True, 0, -1, float(FLOOR), str(FLOOR), 1 << 64):
                with self.subTest(field=key, value=value):
                    self.assertTrue(self.resident_issues({RSS: resident_contract(**{key: value})},
                                                         {"resident_policies": resident_policy(**{key: value})}))
        inverted = {"hard_ceiling_bytes": FLOOR, "comparison_allowance_floor_bytes": FLOOR + 1}
        self.assertTrue(self.resident_issues({RSS: resident_contract(**inverted)},
                                             {"resident_policies": resident_policy(**inverted)}))
        for key, value in (("quantum", 1), ("resolution", 4096), ("resolution_method", "page size")):
            with self.subTest(timer_field=key):
                self.assertTrue(self.resident_issues({RSS: resident_contract(**{key: value})},
                                                     {"resident_policies": resident_policy()}))

    def test_no_selected_resident_metric_needs_no_policy_and_refuses_a_fabricated_one(self):
        self.assertEqual(self.resident_issues({}, {}), [])
        self.assertTrue(self.resident_issues({}, {"resident_policies": resident_policy()}))
        self.assertTrue(self.resident_issues({}, {"resident_policies": {}}))

    def test_previous_method_kind_has_no_resource_formula_fallback(self):
        issues = self.resident_issues({RSS: resident_contract()}, {"resident_policies": resident_policy()},
                                      kind="consumer-method-review-v1")
        self.assertIn(collector.issue("admission", "accepted consumer method review required"), issues)


class TrialAdmission(BoundReview):
    def trial_issues(self, metrics, *resources, kind=collector.METHOD_REVIEW):
        return self.review_issues(metrics, *resources, kind=kind, prefixes=tuple(name + ":" for name in metrics))

    @staticmethod
    def declared(**changes):
        return {"trial_responsiveness": trial_declaration(**changes)}

    def test_matching_bound_budget_admits_trial_operation(self):
        self.assertEqual(self.trial_issues({OP: trial_contract()}, self.declared()), [])
        # A bound resources record without a declaration neither satisfies nor contradicts another.
        self.assertEqual(self.trial_issues({OP: trial_contract()}, {}, self.declared()), [])
        equal = {"max_regression_us": TRIAL_CEILING}
        self.assertEqual(self.trial_issues({OP: trial_contract(**equal)}, self.declared(**equal)), [])

    def test_response_ceiling_must_be_below_process_timeout(self):
        timeout_us = self.plan["bounds"]["primary_seconds"] * 1_000_000
        for ceiling, accepted in ((timeout_us - 1, True), (timeout_us, False), (timeout_us + 1, False)):
            with self.subTest(ceiling=ceiling):
                issues = self.trial_issues({OP: trial_contract(hard_ceiling_us=ceiling)},
                                          self.declared(hard_ceiling_us=ceiling))
                self.assertEqual(not issues, accepted)
                if not accepted:
                    self.assertTrue(any("outer process timeout" in row["reason"] for row in issues))

    def test_missing_mismatched_unbound_or_inconsistent_declaration_refuses(self):
        declared = self.declared()
        cases = {
            "missing": [{}],
            "mismatched ceiling": [self.declared(hard_ceiling_us=TRIAL_CEILING + 1)],
            "mismatched tolerance": [self.declared(max_regression_us=TRIAL_REGRESSION - 1)],
            "other policy": [self.declared(policy="fresh-trial-responsiveness-v0")],
            "other population": [self.declared(population="warm-fresh-process")],
            "inconsistent second record": [declared, self.declared(max_regression_us=TRIAL_REGRESSION + 1)],
            "other context": [{**declared, "context_sha256": "0" * 64}],
            "unbound source": [{**declared, "source_files": {self.source: "0" * 64}}],
            "empty justification": [self.declared(justification=" ")],
            "not an object": [{"trial_responsiveness": [trial_declaration()]}],
        }
        for name, resources in cases.items():
            with self.subTest(case=name):
                self.assertTrue(self.trial_issues({OP: trial_contract()}, *resources))

    def test_invalid_budget_fields_refuse_even_when_declaration_repeats_them(self):
        for key in ("hard_ceiling_us", "max_regression_us"):
            for value in (None, True, 0, -1, float(TRIAL_REGRESSION), TRIAL_REGRESSION + 0.5,
                          str(TRIAL_REGRESSION), 1 << 64):
                with self.subTest(field=key, value=value):
                    self.assertTrue(self.trial_issues({OP: trial_contract(**{key: value})},
                                                      self.declared(**{key: value})))
        inverted = {"hard_ceiling_us": TRIAL_REGRESSION, "max_regression_us": TRIAL_REGRESSION + 1}
        self.assertTrue(self.trial_issues({OP: trial_contract(**inverted)}, self.declared(**inverted)))
        for key in ("policy", "population"):
            with self.subTest(missing=key):
                self.assertTrue(self.trial_issues({OP: trial_contract(**{key: None})}, self.declared(**{key: None})))

    def test_clock_must_be_positive_and_twice_resolution_must_fit_tolerance(self):
        self.assertEqual(self.trial_issues({OP: trial_contract(resolution=TRIAL_REGRESSION // 2)}, self.declared()), [])
        for changes in ({"resolution": f"{TRIAL_REGRESSION + 1}/2"}, {"resolution": 0}, {"quantum": 0},
                        {"quantum": True}, {"resolution_method": " "}):
            with self.subTest(changes=changes):
                self.assertTrue(self.trial_issues({OP: trial_contract(**changes)}, self.declared()))

    def test_trial_policy_is_refused_outside_trial_operation(self):
        issues = self.trial_issues({OP: trial_contract(), RSS: resident_contract(**trial_budget())},
                                   {**self.declared(), "resident_policies": resident_policy()})
        self.assertTrue(any(row["reason"].startswith(RSS + ":") for row in issues))
        self.plan["case"] = "group-full"
        self.context = collector.context_digest(self.plan)
        group = {"api": "host call records", "owner": "engine", "interval": "first recognize to last release",
                 "unit": "ns", "missing_reason": None, "quantum": 1, "resolution": 1, "resolution_method": "source-bound"}
        self.assertEqual(self.trial_issues({"group_span_ns": group}, {}), [])
        self.assertTrue(self.trial_issues({"group_span_ns": {**group, **trial_budget()}}, {}))
        self.assertTrue(self.trial_issues({"group_span_ns": group}, self.declared()))

    def test_previous_method_kinds_are_refused_not_reinterpreted(self):
        for kind in ("consumer-method-review-v1", "consumer-method-review-v2",
                     "consumer-method-review-v3", "consumer-method-review-v4"):
            with self.subTest(kind=kind):
                self.assertIn(collector.issue("admission", "accepted consumer method review required"),
                              self.trial_issues({OP: trial_contract()}, self.declared(), kind=kind))


class ResidentGrading(unittest.TestCase):
    BASE = 100 * collector.MIB

    def test_declared_ceiling_and_floor_are_not_sample_derived(self):
        self.assertEqual(calibrated_resident(), {
            "resource": True, "ceiling": str(HARD), "comparison_allowance_floor_bytes": str(FLOOR),
            "allowance": str(FLOOR), "spread": "0", "median": str(self.BASE),
            "minimum": str(self.BASE), "maximum": str(self.BASE)})
        wider = calibrated_resident(hard_ceiling_bytes=2 * HARD, comparison_allowance_floor_bytes=1)
        self.assertEqual((wider["ceiling"], wider["allowance"]), (str(2 * HARD), "1"))

    def test_spread_above_floor_and_half_byte_median_stay_exact(self):
        values = [self.BASE] * 3 + [self.BASE + 1] + [self.BASE + 3 * collector.MIB] * 2
        envelope = calibrated_resident(values)
        self.assertEqual(envelope["allowance"], str(3 * collector.MIB))
        self.assertEqual(envelope["median"], str(collector.Fraction(2 * self.BASE + 1, 2)))
        self.assertEqual(envelope["comparison_allowance_floor_bytes"], str(FLOOR))

    def test_calibration_repeatability_boundary_and_absolute_ceiling(self):
        unit = 1024
        self.assertEqual(calibrated_resident([925 * unit] + [1000 * unit] * 4 + [1075 * unit])["spread"],
                         str(150 * unit))
        with self.assertRaisesRegex(ValueError, "repeatability"):
            calibrated_resident([924 * unit] + [1000 * unit] * 4 + [1075 * unit])
        self.assertEqual(calibrated_resident([HARD] * 6)["maximum"], str(HARD))
        for bad in (HARD + 1, 0, str(self.BASE), float(self.BASE), True):
            with self.subTest(value=bad), self.assertRaises(ValueError):
                calibrated_resident([bad] + [self.BASE] * 5)
        with self.assertRaisesRegex(ValueError, "unavailable calibration"):
            calibrated_resident([None] + [self.BASE] * 5)

    def test_every_invocation_obeys_fixed_point_ceiling_and_missing_stays_unavailable(self):
        with tempfile.TemporaryDirectory() as directory:
            plan = plan_for(directory)
            output = trial_output(plan)
            slot = collector.schedule("calibration")[0]
            review = lambda ceiling: {"metrics": {RSS: resident_contract(hard_ceiling_bytes=ceiling)}}
            result = collector.reconcile(plan, slot, record_for(output), review(self.BASE))
            self.assertEqual(result["issues"], [])
            self.assertEqual(result["metrics"][RSS], self.BASE)
            result = collector.reconcile(plan, slot, record_for(output), review(self.BASE - 1))
            self.assertEqual(result["issues"],
                             [collector.issue("numerical", RSS + " exceeds fixed resident checkpoint ceiling")])
            output["operation"]["controller"]["result"]["measurement"].pop("process_rss_after")
            result = collector.reconcile(plan, slot, record_for(output), review(self.BASE - 1))
            self.assertEqual(result["issues"], [])
            self.assertIn(collector.issue("unavailable", "required metric missing: " + RSS), result["unavailable"])

    def test_unchanged_resident_control_uses_balanced_checks(self):
        envelopes = {RSS: calibrated_resident()}

        def grade(rows):
            return [row["reason"] for row in collector.grade_rows(rows, "control", envelopes, "trial-hit-roi")]

        self.assertEqual(grade(cohort_rows("control", RSS, self.BASE, self.BASE + FLOOR)), [])
        self.assertTrue(any("role bias" in reason for reason in
                            grade(cohort_rows("control", RSS, self.BASE, self.BASE + FLOOR + 1))))
        for name, shifted in (("order", lambda slot: slot["position"] == 1),
                              ("drift", lambda slot: slot["process"] >= 3)):
            rows = cohort_rows("control", RSS, self.BASE, self.BASE)
            for row in rows:
                if shifted(row["slot"]):
                    row["metrics"][RSS] += FLOOR + 1
            with self.subTest(check=name):
                self.assertTrue(any(name in reason for reason in grade(rows)))
        rows = cohort_rows("control", RSS, self.BASE, self.BASE)
        rows[0]["metrics"][RSS] += 16 * collector.MIB
        self.assertTrue(any("repeatability" in reason for reason in grade(rows)))

    def test_candidate_resident_median_is_non_regression_not_benefit(self):
        def grade(case, metric, envelope, timing, resident):
            rows = paired_rows("comparison", metric, timing, resident)
            envelopes = {metric: envelope, RSS: calibrated_resident()}
            return [row["reason"] for row in collector.grade_rows(rows, "comparison", envelopes, case)]

        trial = ("trial-hit-roi", OP, calibrated_trial())
        self.assertEqual(grade(*trial, (1000, 1000), (self.BASE, self.BASE + FLOOR)), [])
        self.assertEqual(grade(*trial, (1000, 1000), (self.BASE, self.BASE + FLOOR + 1)),
                         [RSS + ": candidate resident median exceeds baseline plus allowance"])
        # A resident improvement never substitutes for the group benefit it does not measure.
        group = ("group-full", "group_span_ns", collector.scalar_envelope([1000] * 6, 1, 1))
        self.assertEqual(grade(*group, (1000, 997), (self.BASE, self.BASE + FLOOR)), [])
        self.assertEqual(grade(*group, (1000, 1000), (self.BASE, self.BASE - 10 * collector.MIB)),
                         ["group_span_ns: scalar benefit does not exceed allowance"])

    def test_per_process_hard_ceiling_survives_passing_medians(self):
        rows = cohort_rows("comparison", RSS, self.BASE, self.BASE)
        rows[5]["metrics"][RSS] = HARD + 1
        rows[6]["metrics"].pop(RSS)
        issues = collector.grade_rows(rows, "comparison", {RSS: calibrated_resident()}, "trial-hit-roi")
        self.assertIn(collector.issue("numerical", "05: " + RSS + " exceeds frozen ceiling"), issues)
        self.assertIn(collector.issue("unavailable", "06: " + RSS), issues)


class TrialGrading(unittest.TestCase):
    """Synthetic budget arithmetic only; no historical cohort is regraded."""

    def grade(self, stage, rows, envelope=None):
        return collector.grade_rows(rows, stage, {OP: envelope or calibrated_trial()}, "trial-hit-roi")

    @staticmethod
    def pair_rows(stage, baseline, other):
        """Unchanged roles except corresponding pair 0, which carries the given values."""
        rows = cohort_rows(stage, OP, 1000, 1000)
        for row in rows:
            if row["slot"]["pair"] == 0:
                row["metrics"][OP] = baseline if row["slot"]["role"] == "baseline" else other
        return rows

    def test_declared_ceiling_and_tolerance_are_not_sample_derived(self):
        values = [990, 995, 1000, 1000, 1005, 1010]
        envelope = calibrated_trial(values)
        self.assertEqual((envelope["ceiling"], envelope["allowance"]), (str(TRIAL_CEILING), str(TRIAL_REGRESSION)))
        self.assertEqual((envelope["spread"], envelope["central_four"]["spread"]), ("20", "10"))
        self.assertEqual(envelope["central_four"]["values"], ["995", "1000", "1000", "1005"])
        values[4] += 1
        with self.assertRaises(collector.ContractError) as caught:
            calibrated_trial(values)
        self.assertEqual(caught.exception.category, "inconclusive")

    def test_calibration_uses_absolute_tolerance_not_percentage_gate(self):
        # 50% range/median: refused by the scalar gate, within the declared absolute tolerance.
        relative = [10, 20, 20, 20, 20, 20]
        self.assertEqual(calibrated_trial(relative)["spread"], "10")
        with self.assertRaisesRegex(ValueError, "noise"):
            collector.scalar_envelope(relative, 1, 1)
        # Two high values leave one inside the central four; the absolute tolerance still applies.
        absolute = [1000, 1000 + TRIAL_REGRESSION + 1, 1000 + TRIAL_REGRESSION + 1, 1000, 1000, 1000]
        collector.scalar_envelope(absolute, 1, 1)
        with self.assertRaises(collector.ContractError) as caught:
            calibrated_trial(absolute)
        self.assertEqual(caught.exception.category, "inconclusive")

    def test_calibration_ceiling_completeness_and_clock_are_enforced(self):
        self.assertEqual(calibrated_trial([TRIAL_CEILING] * 6)["maximum"], str(TRIAL_CEILING))
        with self.assertRaisesRegex(ValueError, "ceiling") as caught:
            calibrated_trial([TRIAL_CEILING + 1] + [TRIAL_CEILING] * 5)
        self.assertNotIsInstance(caught.exception, collector.ContractError)
        with self.assertRaisesRegex(ValueError, "unavailable calibration"):
            calibrated_trial([None] + [1000] * 5)
        with self.assertRaisesRegex(ValueError, "six"):
            collector.derive_envelopes(cohort_rows("calibration")[:5], {"metrics": {OP: trial_contract()}}, "trial-hit-roi")
        self.assertEqual(calibrated_trial(resolution=TRIAL_REGRESSION // 2)["resolution"], str(TRIAL_REGRESSION // 2))
        for changes in ({"resolution": f"{TRIAL_REGRESSION + 1}/2"}, {"quantum": 0}, {"resolution": 0},
                        {"policy": None}, {"max_regression_us": TRIAL_CEILING + 1},
                        {"hard_ceiling_us": float(TRIAL_CEILING)}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                calibrated_trial(**changes)

    def test_unchanged_control_pairs_and_role_medians_share_the_tolerance(self):
        self.assertEqual(self.grade("control", cohort_rows("control", OP, 1000, 1000 + TRIAL_REGRESSION)), [])
        issues = self.grade("control", cohort_rows("control", OP, 1000, 1000 + TRIAL_REGRESSION + 1))
        self.assertEqual({row["category"] for row in issues}, {"inconclusive"})
        self.assertEqual(sum("A/A pair" in row["reason"] for row in issues), 6)
        self.assertIn(collector.issue(
            "inconclusive", OP + ": unchanged A/A role median difference exceeds max_regression_us"), issues)
        pair = [collector.issue("inconclusive", OP + ": unchanged A/A pair 0 difference exceeds max_regression_us")]
        self.assertEqual(self.grade("control", self.pair_rows("control", 995, 1005)), [])
        self.assertEqual(self.grade("control", self.pair_rows("control", 995, 1006)), pair)
        self.assertEqual(self.grade("control", self.pair_rows("control", 1006, 995)), pair)

    def test_role_range_order_and_drift_are_inconclusive_not_regression(self):
        for name, shifted in (("central-four range", lambda slot: slot["pair"] in (1, 2)),
                              ("order", lambda slot: slot["position"] == 1),
                              ("drift", lambda slot: slot["process"] >= 3)):
            rows = cohort_rows("control", OP, 1000, 1000)
            for row in rows:
                if shifted(row["slot"]):
                    row["metrics"][OP] += TRIAL_REGRESSION + 1
            issues = self.grade("control", rows)
            with self.subTest(check=name):
                self.assertEqual({row["category"] for row in issues}, {"inconclusive"})
                for role in ("baseline", "control"):
                    self.assertIn(collector.issue("inconclusive", f"{role}: {OP} {name} exceeds max_regression_us"), issues)
        # The same movement in both comparison roles is uncertainty, not an observed regression.
        rows = cohort_rows("comparison", OP, 1000, 1000)
        for row in rows:
            if row["slot"]["process"] >= 3:
                row["metrics"][OP] += TRIAL_REGRESSION + 1
        self.assertEqual({row["category"] for row in self.grade("comparison", rows)}, {"inconclusive"})

    def test_comparison_budget_violation_is_numerical_and_faster_needs_no_gain(self):
        self.assertIsNone(collector.benefit_metric("trial-hit-roi"))
        for other in (500, 1000, 1000 + TRIAL_REGRESSION):
            with self.subTest(candidate=other):
                self.assertEqual(self.grade("comparison", cohort_rows("comparison", OP, 1000, other)), [])
        issues = self.grade("comparison", cohort_rows("comparison", OP, 1000, 1000 + TRIAL_REGRESSION + 1))
        self.assertEqual({row["category"] for row in issues}, {"numerical"})
        self.assertEqual(sum(" pair " in row["reason"] for row in issues), 6)
        self.assertIn(collector.issue(
            "numerical", OP + ": candidate median exceeds baseline median plus max_regression_us"), issues)
        self.assertEqual(self.grade("comparison", self.pair_rows("comparison", 995, 1005)), [])
        self.assertEqual(self.grade("comparison", self.pair_rows("comparison", 995, 1006)),
                         [collector.issue("numerical", OP + ": pair 0 candidate exceeds baseline plus max_regression_us")])
        self.assertEqual(self.grade("comparison", self.pair_rows("comparison", 1006, 995)), [])

    def test_inconclusive_noise_does_not_erase_known_violation(self):
        rows = cohort_rows("comparison", OP, 1000, 1000 + TRIAL_REGRESSION + 1)
        for row in rows:
            if row["slot"]["role"] == "candidate" and row["slot"]["pair"] in (2, 3):
                row["metrics"][OP] += TRIAL_REGRESSION + 1
        issues = self.grade("comparison", rows)
        self.assertIn(collector.issue("inconclusive", f"candidate: {OP} central-four range exceeds max_regression_us"), issues)
        self.assertIn(collector.issue(
            "numerical", OP + ": candidate median exceeds baseline median plus max_regression_us"), issues)

    def test_one_ceiling_violation_survives_passing_pairs_and_medians(self):
        envelope = calibrated_trial(hard_ceiling_us=1005)
        rows = cohort_rows("comparison", OP, 1000, 1000)
        rows[5]["metrics"][OP] = 1006
        self.assertEqual(self.grade("comparison", rows, envelope),
                         [collector.issue("numerical", "05: " + OP + " exceeds frozen ceiling")])
        rows[6]["metrics"].pop(OP)
        self.assertIn(collector.issue("unavailable", "06: " + OP), self.grade("comparison", rows, envelope))

    def test_complete_pair_violation_survives_other_missing_or_unrun_slots(self):
        for stage, category in (("control", "inconclusive"), ("comparison", "numerical")):
            for incomplete in ("missing", "unrun"):
                with self.subTest(stage=stage, incomplete=incomplete):
                    rows = self.pair_rows(stage, 1000, 1000 + TRIAL_REGRESSION + 1)
                    if incomplete == "missing":
                        rows[-2]["metrics"][OP] = None
                    else:
                        rows = rows[:2]
                    issues = self.grade(stage, rows)
                    self.assertTrue(any(row["category"] == category and "pair 0" in row["reason"]
                                        for row in issues))
                    self.assertIn("unavailable" if incomplete == "missing" else "incomplete",
                                  {row["category"] for row in issues})
                    self.assertFalse(any("median" in row["reason"] for row in issues))


class TrialCentralFour(unittest.TestCase):
    def test_ties_remove_exactly_one_occurrence_at_each_tail(self):
        result = collector.trial_central_four([105, 100, 110, 100, 110, 105])
        self.assertEqual(result["input_indices"], [3, 0, 5, 2])
        self.assertEqual([(row["side"], row["input_index"]) for row in result["excluded"]],
                         [("minimum", 1), ("maximum", 4)])
        self.assertEqual(result["values"], ["100", "105", "105", "110"])
        equal = collector.trial_central_four([1000] * 6)
        self.assertEqual(equal["input_indices"], [1, 2, 3, 4])
        self.assertEqual([row["input_index"] for row in equal["excluded"]], [0, 5])
        self.assertEqual((equal["spread"], equal["mean"]), ("0", "1000"))

    def test_central_mean_remains_exact_above_float_integer_precision(self):
        base = 9007199254740993
        result = collector.trial_central_four([base - 100, base, base + 1, base + 2, base + 3, base + 100])
        self.assertEqual(result["mean"], str(collector.Fraction(2 * base + 3, 2)))
        self.assertEqual(result["median"], result["mean"])
        self.assertEqual(result["spread"], "3")

    def test_extreme_is_not_an_automatic_first_invocation_exclusion(self):
        rows = cohort_rows("calibration")
        for row, value in zip(rows, [1002, 990, 1001, 1003, 1010, 1004]):
            row["metrics"][OP] = value
        statistics = collector.cohort_statistics(rows, "trial-hit-roi", central_four=True)["baseline"][OP]
        central = statistics["central_four"]
        self.assertEqual([statistics["slots"][row["input_index"]]["id"] for row in central["excluded"]], ["01", "04"])
        self.assertIn("00", [statistics["slots"][index]["id"] for index in central["input_indices"]])
        self.assertEqual(statistics["values"], ["1002", "990", "1001", "1003", "1010", "1004"])
        self.assertEqual((statistics["spread"], central["spread"], central["mean"]), ("20", "3", "2005/2"))
        self.assertEqual(statistics["median"], central["median"])

    def test_unadmitted_statistics_retain_full_values_without_successor_projection(self):
        rows = cohort_rows("calibration")
        rows[0]["metrics"][OP] = 1100
        historical = collector.cohort_statistics(rows, "trial-hit-roi")["baseline"][OP]
        self.assertEqual(historical["values"], ["1100", "1000", "1000", "1000", "1000", "1000"])
        self.assertEqual(historical["spread"], "100")
        self.assertNotIn("central_four", historical)
        admitted = collector.cohort_statistics(rows, "trial-hit-roi", central_four=True)["baseline"][OP]
        self.assertEqual(admitted["central_four"]["spread"], "0")
        self.assertEqual(admitted["values"], historical["values"])

    def test_incomplete_values_never_form_a_replacement_central_cohort(self):
        for values in ([1, 2, 3, 4, 5], [1, 2, 3, 4, 5, 6, 7], [1, 2, 3, 4, 5, None]):
            with self.subTest(values=values), self.assertRaises(ValueError):
                collector.trial_central_four(values)
        for unrun in (False, True):
            rows = cohort_rows("calibration")
            if unrun:
                rows.pop()
            else:
                rows[0]["metrics"][OP] = None
            self.assertIsNone(collector.cohort_statistics(rows, "trial-hit-roi", central_four=True)["baseline"][OP]["central_four"])

    def test_a_trimmed_extreme_still_obeys_ceilings_and_original_pairs(self):
        envelope = calibrated_trial([1000] * 5 + [1100])
        self.assertEqual((envelope["spread"], envelope["central_four"]["spread"]), ("100", "0"))
        rows = cohort_rows("comparison", OP, 1000, 1000)
        rows[1]["metrics"][OP] = 1100
        self.assertEqual(collector.cohort_statistics(rows, "trial-hit-roi", central_four=True)["candidate"][OP]["central_four"]["spread"], "0")
        issues = collector.grade_rows(rows, "comparison", {OP: envelope}, "trial-hit-roi")
        self.assertIn(collector.issue("numerical", OP + ": pair 0 candidate exceeds baseline plus max_regression_us"), issues)
        rows[1]["metrics"][OP] = TRIAL_CEILING + 1
        issues = collector.grade_rows(rows, "comparison", {OP: envelope}, "trial-hit-roi")
        self.assertIn(collector.issue("numerical", "01: " + OP + " exceeds frozen ceiling"), issues)
        rows = cohort_rows("control", OP, 1000, 1000)
        rows[1]["metrics"][OP] = 1100
        issues = collector.grade_rows(rows, "control", {OP: envelope}, "trial-hit-roi")
        self.assertIn(collector.issue("inconclusive", OP + ": unchanged A/A pair 0 difference exceeds max_regression_us"), issues)
        rows = cohort_rows("comparison", OP, 1000, 1000)
        rows[0]["metrics"][OP], rows[1]["metrics"][OP] = 1100, 1100 + TRIAL_REGRESSION
        self.assertEqual(collector.grade_rows(rows, "comparison", {OP: envelope}, "trial-hit-roi"), [])
        rows[1]["metrics"][OP] += 1
        self.assertIn(collector.issue("numerical", OP + ": pair 0 candidate exceeds baseline plus max_regression_us"),
                      collector.grade_rows(rows, "comparison", {OP: envelope}, "trial-hit-roi"))

    def test_other_metrics_do_not_acquire_trimmed_grading(self):
        with self.assertRaises(ValueError):
            collector.scalar_envelope([1000] * 5 + [1200], 1, 1)
        with self.assertRaises(ValueError):
            calibrated_resident([ResidentGrading.BASE] * 5 + [ResidentGrading.BASE + 16 * collector.MIB])
        rows = cohort_rows("calibration", "group_span_ns", 1000, 1000)
        self.assertNotIn("central_four", collector.cohort_statistics(rows, "group-full", central_four=True)["baseline"]["group_span_ns"])


if __name__ == "__main__":
    unittest.main()
