"""Synthetic evaluator protocol tests only: these are never measurements or host budgets."""
from __future__ import annotations

import base64
import copy
import hashlib
import json
import math
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import report

RUN = Path(report.__file__).with_name("run.py")
REFERENCE, CANDIDATE = "synthetic-reference", "synthetic-candidate"
APPLE, WINDOWS = report.CRITERIA["release_targets"]


def git(root, *arguments):
    return subprocess.run(["git", "-C", str(root), *arguments], check=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, encoding="utf-8").stdout.strip()


def policy_evidence(value, revision, path):
    raw = report.document_bytes(value)
    return {"base64": base64.b64encode(raw).decode("ascii"),
            "sha256": hashlib.sha256(raw).hexdigest(), "revision": revision, "path": path,
            "git_blob": hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest()}


def policy_documents(value):
    return {"budgets": {k: v for k, v in value["budgets"].items() if k != "frozen_revision"},
            "host_policy": value["host_policy"]}


def bind_commit(value, revision, tree, *, update_build=True):
    binding = value["binding"]
    binding.update(source_revision=revision, source_tree=tree, frozen_revision=revision)
    if update_build:
        binding.update(build_source_revision=revision, build_source_tree=tree,
                       build_compiler=binding["compiler"], build_source_dirty=False)
    for processes in value["profiles"].values():
        for process in processes:
            process["binding"] = copy.deepcopy(binding)
            if update_build:
                process["build"] = {key: binding[key] for key in report.BUILD_FIELDS}
    if update_build:
        for command in value["commands"]:
            description = json.loads(command["process"]["stdout"])
            description["build"] = {key: binding[key] for key in report.BUILD_FIELDS}
            command["process"]["stdout"] = json.dumps(description)


def freeze_policies(root, values):
    for value in values:
        for key, document in policy_documents(value).items():
            path = root / "policies" / f"{value['binding']['target']}-{key}.json"
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(report.document_bytes(document))
    git(root, "add", ".")
    git(root, "-c", "user.name=Synthetic Evidence", "-c", "user.email=synthetic@example.invalid",
        "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "freeze synthetic policy inputs")
    revision, tree = git(root, "rev-parse", "HEAD"), git(root, "rev-parse", "HEAD^{tree}")
    for value in values:
        bind_commit(value, revision, tree)
        value["budgets"]["frozen_revision"] = revision
        value["policy_inputs"] = {key: policy_evidence(document, revision,
            f"policies/{value['binding']['target']}-{key}.json") for key, document in policy_documents(value).items()}
        value["budget_sha256"] = value["policy_inputs"]["budgets"]["sha256"]
        value["host_policy_sha256"] = value["policy_inputs"]["host_policy"]["sha256"]


def retain_raw(value, root):
    root.mkdir(exist_ok=True)
    for profile, processes in value["profiles"].items():
        for process in processes:
            keys = ("index", "pid", "binding", "profile", "identity", "build", "target", "opencv", "runtime", "provider")
            records = [dict({key: process[key] for key in keys}, type="meta"),
                       dict(process["collector_scope"], type="collector-scope"),
                       {"type": "vocabulary", "supported": process["vocabulary_supported"]}]
            records.extend(dict(row, type="sample") for row in process["samples"])
            records.extend({"type": "workload", "workload": name, "metrics": metrics}
                           for name, metrics in process["workloads"].items())
            records.extend(process["raw_records"])
            records.append({key: process[key] for key in ("lifecycle", "lifecycle_detail", "metrics", "close_fault")})
            records[-1]["type"] = "terminal"
            path = root / f"{profile}-{process['index']}.jsonl"
            path.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in records), encoding="utf-8")
            process["raw_path"], process["raw_sha256"] = path.name, report.sha256(path)
            receipt_path = root / f"{profile}-{process['index']}.process.json"
            receipt_path.write_bytes(report.document_bytes(
                {key: process[key] for key in ("index", "process", "command")}))
            process.update(receipt_path=receipt_path.name, receipt_sha256=report.sha256(receipt_path))


def synthetic_report(target=APPLE, *, schema_version=2, accepted=(True, False)):
    """Schema 1 reproduces the historical runner's identity ``accepted`` field and command mode."""
    fixtures = report.frozen_corpus()
    for row in fixtures:
        row["file"] = f"/synthetic/licensed/{row['id']}.png"
    binding = {key: f"synthetic-{key}" for key in report.IDENTITY_FIELDS + report.COHORT_FIELDS}
    for key in ("source_revision", "source_tree"):
        binding[key] = "1" * 40
    for key in ("executable_sha256", "runtime_sha256", "catalog_sha256"):
        binding[key] = "2" * 64
    binding.update({"criteria_sha256": report.sha256(Path(report.__file__).with_name("criteria-v1.json")),
                    "fixtures_sha256": hashlib.sha256(report.document_bytes({"fixtures": fixtures})).hexdigest(),
                    "target": target, "os": "macOS 15.0" if target == APPLE else "Windows-11-synthetic",
                    "provider": "cpu", "runtime": "1.29.0-api17", "intra_op": 1, "inter_op": 1,
                    "qualified_host": True, "host_approval": "synthetic-test-only", "serial_execution": True,
                    "frozen_revision": "3" * 40})
    binding.update(build_source_revision=binding["source_revision"], build_source_tree=binding["source_tree"],
                   build_compiler=binding["compiler"], build_source_dirty=False)
    limits = {"timeout_seconds": 1000, "output_limit_bytes": 100000000, "cleanup_seconds": 5, "lifecycle_bound_ms": 5000}
    mode_flag = "--model-admission" if schema_version == 1 else "--model-evaluation"
    value = {"schema_version": schema_version, "criteria_id": report.CRITERIA["criteria_id"], "binding": binding,
             "fixtures": fixtures, "corpora": report.corpus_identities(),
             "reference_profile": REFERENCE, "candidate_profile": CANDIDATE,
             "identities": {}, "profiles": {}, "commands": [], "preflight_failures": [],
             "diagnostic": False, "process_limits": limits,
             "execution": report.COMPLETE_EXECUTION[schema_version],
             "reproduction_command": ["/synthetic/private/python", "run.py", "run"]}
    cohort = {key: binding[key] for key in report.COHORT_FIELDS}
    value["host_policy"] = {"approved_cohorts": [{"cohort": cohort, "approval": binding["host_approval"]}]}
    # Deliberately artificial ceilings, never usable as a qualified-host budget file.
    value["budgets"] = {"approval": "synthetic-test-only", "frozen_revision": binding["frozen_revision"],
                        "cohort": cohort, "process": {key: 10**9 for key in report.BUDGET_PROCESS_METRICS},
                        "workloads": {}, "process_limits": limits,
                        "required_collectors": ["rust-accounting", "native-peak-rss", "onnx-stage-clocks"]}
    good_process = {"exit_code": 0, "timed_out": False, "output_limited": False,
                    "cleanup_ok": True, "launch_error": None, "stdout": "", "stderr": "", "duration_seconds": 1}
    build = {key: binding[key] for key in report.BUILD_FIELDS}
    for (profile, elapsed), historical_accepted in zip(((REFERENCE, 100), (CANDIDATE, 80)), accepted):
        identity = {"profile_id": profile, "model_id": profile, "model_version": "synthetic-v1",
                    "preprocessing_id": "synthetic", "decoder_id": "synthetic", "normalization_id": "synthetic",
                    "execution_sha256": "5" * 64,
                    "limits": {"detector_width": 736, "detector_height": 736, "tensor_bytes": 256 * 1024 * 1024},
                    "resources": [{"kind": kind, "relative_path": kind + ".onnx", "bytes": 10, "sha256": "6" * 64}
                                  for kind in ("detector", "recognizer")]}
        if schema_version == 1:
            identity["accepted"] = historical_accepted
        value["identities"][profile] = identity
        descriptor_process = dict(good_process, stdout=json.dumps({"identity": identity, "build": build}))
        value["commands"].append({"command": ["synthetic-onnx-cpu", mode_flag, "--describe-profile", "--profile", profile], "process": descriptor_process})
        processes = []
        for index in range(3):
            process = {"index": index, "pid": 100 + index + (0 if profile == REFERENCE else 10), "binding": copy.deepcopy(binding),
                       "profile": profile, "identity": copy.deepcopy(identity), "build": copy.deepcopy(build),
                       "target": target, "opencv": binding["opencv"], "runtime": binding["runtime"], "provider": "cpu",
                       "vocabulary_supported": True, "process": dict(good_process),
                       "command": ["synthetic-onnx-cpu", mode_flag, "--profile", profile, "--process-index", str(index)],
                       "metrics": {"cold_open_ms": 1, "close_ms": 1, "peak_resident_bytes": 100, "model_bytes": 20, "auxiliary_bytes": 0},
                       "lifecycle": dict(report.CRITERIA["required_lifecycle"], initialization_cancelled="Cancelled"),
                       "samples": [], "workloads": {}, "close_fault": None, "raw_records": []}
            process["collector_scope"] = dict(report.COLLECTOR_SCOPE)
            process["lifecycle_detail"] = {kind: {"scope": "admitted-before-native-entry", "admitted": True,
                "termination_issued": True, "worker_joined": True, "return_ms": 1, "bound_ms": 5000,
                "cleanup_completions": 1} for kind in ("cancelled", "deadline")}
            for fixture in fixtures:
                for mode in ("singular", "grouped"):
                    name = f"{fixture['id']}/{mode}"
                    value["budgets"]["workloads"][name] = {key: 10**9 for key in report.BUDGET_WORKLOAD_METRICS}
                    process["workloads"][name] = {"rust_peak_allocated_bytes": 100, "rust_growth_bytes": 0, "rust_steady_bytes": 0}
                    actual = [{"text": region["text_nfc"], "quad": copy.deepcopy(region["source_relative_quad"]), "confidence": 0.1}
                              for region in fixture["regions"]]
                    groups = [list(range(len(actual))), list(range(len(actual)))] if mode == "grouped" else None
                    source = {key: "synthetic-" + key for key in ("stamp", "transform", "effective_region", "backend", "effective_zones")}
                    source["output_space"] = "CapturePixels"
                    for phase, count in (("warmup", 2), ("sample", 20)):
                        for sample_index in range(count):
                            metrics = {key: 1 for key in report.CRITERIA["required_sample_metrics"]}
                            metrics.update({"elapsed_ms": elapsed, "mapped_bytes": fixture["width"] * fixture["height"] * 4,
                                            "detector_width": 32, "detector_height": 32, "detector_tensor_bytes": 32 * 32 * 3 * 4,
                                            "detector_runs": 1, "recognizer_runs": math.ceil(len(actual) / 6)})
                            process["samples"].append({"type": "sample", "workload": name, "phase": phase, "index": sample_index,
                                "status": "Ok", "source_correlation": True, "source": source, "result_source": dict(source),
                                "regions": copy.deepcopy(actual), "groups": groups, "metrics": metrics})
            processes.append(process)
        value["profiles"][profile] = processes
    return value


def rules(scope, profile):
    return {rule["rule_id"]: rule for rule in scope["profiles"][profile]["rules"]}


def outcomes(scope, profile):
    return {rule_id: rule["outcome"] for rule_id, rule in rules(scope, profile).items()}


def expected(*, reference=False, default="passed", **changes):
    result = {rule: default for rule in report.RULE_IDS}
    if reference:
        result["relative_latency"] = "not_run"
    result.update(changes)
    return result


def walk(value):
    if isinstance(value, dict):
        for key, item in value.items():
            yield key
            yield from walk(item)
    elif isinstance(value, list):
        for item in value:
            yield from walk(item)
    else:
        yield value


class EvaluationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.original = synthetic_report()
        cls.original_windows = synthetic_report(WINDOWS)

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ocr-report-evaluation-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.repository = self.root / "repo"
        self.repository.mkdir()
        git(self.repository, "init", "-q", "--object-format=sha1")
        (self.repository / "implementation.rs").write_text("// synthetic implementation A\n", encoding="utf-8")
        self.value = copy.deepcopy(self.original)
        self.windows = copy.deepcopy(self.original_windows)
        self.evidence_root = self.root / "apple"
        self.windows_root = self.root / "windows"
        freeze_policies(self.repository, (self.value, self.windows))

    def evaluate(self, *, retain=True):
        if retain:
            retain_raw(self.value, self.evidence_root)
        return report.evaluate(self.value, evidence_root=self.evidence_root, repository_root=self.repository)

    def write_report(self, value, root):
        retain_raw(value, root)
        path = root / "report.json"
        path.write_bytes(report.document_bytes(value))
        return path

    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, str(RUN), *map(str, arguments)],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def candidate_row(self):
        return self.value["profiles"][CANDIDATE][0]["samples"][2]

    def workload_rows(self, workload, profile=CANDIDATE, value=None):
        return [row for process in (value or self.value)["profiles"][profile]
                for row in process["samples"] if row["workload"] == workload]

    def fail_text(self, value=None):
        """Stable wrong text for region 0 of the first workload in every candidate row."""
        name = self.candidate_row()["workload"]
        for row in self.workload_rows(name, value=value):
            row["regions"][0]["text"] = "wrong"
        return name

    def assert_unverified(self, fragment, *, profiles=(REFERENCE, CANDIDATE), retain=True):
        scope = self.evaluate(retain=retain)
        self.assertFalse(scope["complete"])
        self.assertTrue(any(fragment in error["message"] for error in scope["validation_errors"]),
                        scope["validation_errors"][:10])
        for profile in profiles:
            self.assertNotIn("passed", outcomes(scope, profile).values(), profile)
        return scope

    def test_complete_campaign_reports_rule_outcomes_without_a_selection_decision(self):
        scope = self.evaluate()
        self.assertEqual(scope["validation_errors"], [])
        self.assertTrue(scope["complete"])
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))
        self.assertEqual(rules(scope, REFERENCE)["relative_latency"]["reason"], "comparison_reference")
        # Candidate p50/p95 at exactly 80% of the reference satisfy the unchanged timing rule.
        self.assertEqual(outcomes(scope, CANDIDATE), expected())
        self.assertEqual(scope["profiles"][CANDIDATE]["execution_sha256"], "5" * 64)
        self.assertEqual(scope["scope"]["measured_source"]["revision"], self.value["binding"]["source_revision"])
        terms = set(walk(report.evaluation_bundle([scope])))
        self.assertFalse(terms & {"admitted", "target_qualified", "verdict", "rating", "recommendation",
                                  "fast", "balanced", "unrated"})

    def attempt(self, profile, index, *, process=None):
        source = {key: self.value["binding"][key] for key in
                  ("source_revision", "source_tree", "frozen_revision", "executable_sha256", "catalog_sha256")}
        return {"kind": "evaluation", "profile": profile, "index": index,
                "status": "attempted-unconfirmed" if process is None else "completed",
                "source": source, "binding": copy.deepcopy(self.value["binding"]),
                "identity": copy.deepcopy(self.value["identities"][profile]),
                "command": ["synthetic-onnx-cpu", "--model-evaluation", "--profile", profile,
                            "--process-index", str(index)] if process is None else process["command"],
                "process": None if process is None else process["process"],
                "raw_path": f"{profile}-{index}.jsonl", "receipt_path": f"{profile}-{index}.process.json",
                **({} if process is None else {"receipt_sha256": process["receipt_sha256"]})}

    def test_complete_attempt_checkpoints_preserve_current_evaluation(self):
        retain_raw(self.value, self.evidence_root)
        self.value["attempts"] = [self.attempt(profile, process["index"], process=process)
                                  for profile, processes in self.value["profiles"].items() for process in processes]
        scope = self.evaluate(retain=False)
        self.assertTrue(scope["complete"])
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))
        self.assertEqual(outcomes(scope, CANDIDATE), expected())

    def test_interrupted_attempt_keeps_verified_prefix_quality_and_unknown_child_incomplete(self):
        prefix = self.value["profiles"][REFERENCE][:1]
        prefix[0]["samples"][2]["regions"][0]["text"] = "wrong"
        self.value["profiles"] = {REFERENCE: prefix, CANDIDATE: []}
        retain_raw(self.value, self.evidence_root)
        self.value["attempts"] = [self.attempt(REFERENCE, 0, process=prefix[0]), self.attempt(REFERENCE, 1)]
        self.value["execution"] = "interrupted"
        (self.evidence_root / f"{REFERENCE}-1.jsonl").write_text('{"partial":', encoding="utf-8")
        scope = self.evaluate(retain=False)
        self.assertFalse(scope["complete"])
        self.assertTrue(any("attempted-unconfirmed" in error["message"] for error in scope["validation_errors"]))
        self.assertFalse(any(error["kind"] == "integrity" for error in scope["validation_errors"]))
        self.assertEqual(outcomes(scope, REFERENCE)["text_order"], "failed")
        self.assertEqual(len(scope["profiles"][REFERENCE]["processes"]), 1)
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="not_run"))
        self.assertIsNone(self.value["attempts"][-1]["process"])

    def test_receipt_only_checkpoint_keeps_verified_prefix_quality_failure(self):
        prefix = self.value["profiles"][REFERENCE][:2]
        prefix[0]["samples"][2]["regions"][0]["text"] = "wrong"
        self.value["profiles"] = {REFERENCE: prefix, CANDIDATE: []}
        retain_raw(self.value, self.evidence_root)
        self.value["attempts"] = [
            self.attempt(REFERENCE, process["index"], process=process) for process in prefix
        ]
        self.value["execution"] = "interrupted"
        receipt_only = {key: prefix[1][key] for key in (
            "index", "process", "command", "receipt_path", "receipt_sha256", "raw_path", "raw_sha256"
        )}
        receipt_only["projection_state"] = "receipt-only"
        prefix[1] = receipt_only
        scope = self.evaluate(retain=False)
        self.assertFalse(scope["complete"])
        self.assertFalse(any(error["kind"] == "integrity" for error in scope["validation_errors"]),
                         scope["validation_errors"])
        self.assertEqual(outcomes(scope, REFERENCE)["text_order"], "failed")

    def test_first_unconfirmed_attempt_is_execution_not_an_unattempted_profile(self):
        self.value["profiles"] = {REFERENCE: [], CANDIDATE: []}
        self.evidence_root.mkdir()
        self.value["attempts"] = [self.attempt(REFERENCE, 0)]
        self.value["execution"] = "interrupted"
        scope = self.evaluate(retain=False)
        self.assertFalse(scope["complete"])
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True, default="incomplete"))
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="not_run"))
        self.assertFalse(any(error["kind"] == "integrity" for error in scope["validation_errors"]))

    def test_completed_attempt_must_match_its_independent_receipt_projection(self):
        retain_raw(self.value, self.evidence_root)
        process = self.value["profiles"][REFERENCE][0]
        self.value["attempts"] = [self.attempt(REFERENCE, 0, process=process)]
        self.value["attempts"][0]["receipt_sha256"] = "0" * 64
        self.assert_unverified("completed campaign attempt differs", retain=False)

    def test_quality_failure_is_a_complete_evaluation_with_every_finding(self):
        oracle = self.candidate_row()["regions"][0]["text"]
        name = self.fail_text()
        scope = self.evaluate()
        self.assertEqual(scope["validation_errors"], [])
        self.assertTrue(scope["complete"])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(text_order="failed"))
        findings = rules(scope, CANDIDATE)["text_order"]["findings"]
        self.assertEqual(len(findings), 3 * 22)
        self.assertEqual(findings[0], {"kind": "text", "process": 0, "workload": name, "phase": "warmup",
                                       "sample": 0, "region": 0, "expected": oracle, "observed": "wrong"})
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))

    def test_relative_latency_fails_independently_of_quality(self):
        name = self.fail_text()
        for row in self.workload_rows(name):
            row["metrics"]["elapsed_ms"] = 81
        scope = self.evaluate()
        self.assertTrue(scope["complete"])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(text_order="failed", relative_latency="failed"))
        self.assertEqual([(f["workload"], f["metric"], f["observed"], f["reference"], f["limit"])
                          for f in rules(scope, CANDIDATE)["relative_latency"]["findings"]],
                         [(name, "p50_ms", 81, 100, 80), (name, "p95_ms", 81, 100, 80)])
        self.assertEqual(scope["profiles"][CANDIDATE]["metrics"]["workloads"][name]["p95_ms"], 81)

    def test_quality_failure_does_not_hide_a_passing_timing_rule(self):
        self.fail_text()
        scope = self.evaluate()
        self.assertEqual(rules(scope, CANDIDATE)["relative_latency"]["outcome"], "passed")

    def test_empirical_ceiling_failures_are_findings_of_a_complete_evaluation(self):
        name = self.candidate_row()["workload"]
        self.candidate_row()["metrics"]["elapsed_ms"] = 10**10
        self.value["profiles"][CANDIDATE][1]["metrics"]["peak_resident_bytes"] = 10**10
        scope = self.evaluate()
        self.assertEqual(scope["validation_errors"], [])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(process_budgets="failed", workload_budgets="failed"))
        self.assertEqual(rules(scope, CANDIDATE)["workload_budgets"]["findings"],
                         [{"kind": "ceiling", "process": 0, "workload": name, "metric": "max_ms",
                           "observed": 10**10, "limit": 10**9}])
        self.assertEqual(rules(scope, CANDIDATE)["process_budgets"]["findings"],
                         [{"kind": "ceiling", "process": 1, "metric": "peak_resident_bytes",
                           "observed": 10**10, "limit": 10**9}])

    def test_negative_detection_rules_are_independent(self):
        base = copy.deepcopy(self.value)
        for confidence, changes in ((.5, {"region_count": "failed", "unexpected_detection": "failed"}),
                                    (.49, {"region_count": "failed"})):
            with self.subTest(confidence=confidence):
                self.value = copy.deepcopy(base)
                for row in self.workload_rows(self.candidate_row()["workload"]):
                    row["regions"].append({"text": "●", "confidence": confidence,
                                           "quad": [[.1, .1], [.2, .1], [.2, .2], [.1, .2]]})
                    row["metrics"]["recognizer_runs"] = math.ceil(len(row["regions"]) / 6)
                scope = self.evaluate()
                self.assertTrue(scope["complete"])
                self.assertEqual(outcomes(scope, CANDIDATE), expected(**changes))

    def test_confidence_and_stability_are_separate_rules(self):
        base = copy.deepcopy(self.value)
        name = self.candidate_row()["workload"]
        for case, changes in (("every row", {"confidence": "failed"}), ("one row", {"stability": "failed"})):
            with self.subTest(case=case):
                self.value = copy.deepcopy(base)
                rows = self.workload_rows(name)
                for row in rows if case == "every row" else rows[-1:]:
                    row["regions"][0]["confidence"] = 1.01 if case == "every row" else .2
                scope = self.evaluate()
                self.assertTrue(scope["complete"])
                self.assertEqual(outcomes(scope, CANDIDATE), expected(**changes))

    def test_vocabulary_gap_is_a_quality_finding(self):
        self.value["profiles"][CANDIDATE][1]["vocabulary_supported"] = False
        scope = self.evaluate()
        self.assertTrue(scope["complete"])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(vocabulary_coverage="failed"))
        self.assertEqual(rules(scope, CANDIDATE)["vocabulary_coverage"]["findings"],
                         [{"kind": "unsupported_fixture_text", "process": 1}])

    def test_technical_contract_failures_are_explicit_evaluation_errors(self):
        base = copy.deepcopy(self.value)

        def source(value):
            value["profiles"][CANDIDATE][0]["samples"][2]["result_source"]["stamp"] = "another-frame"

        def lifecycle(value):
            detail = value["profiles"][CANDIDATE][0]["lifecycle_detail"]["cancelled"]
            detail["return_ms"] = detail["bound_ms"] + .001

        def bounds(value):
            value["profiles"][CANDIDATE][0]["samples"][2]["metrics"]["detector_runs"] = 2

        for rule, mutate in (("source_correlation", source), ("lifecycle", lifecycle), ("execution_bounds", bounds)):
            with self.subTest(rule=rule):
                self.value = copy.deepcopy(base)
                mutate(self.value)
                scope = self.evaluate()
                self.assertFalse(scope["complete"])
                self.assertEqual(outcomes(scope, CANDIDATE), expected(**{rule: "failed"}))
                self.assertEqual(scope["validation_errors"], [{"kind": "technical", "profile": CANDIDATE,
                                                              "rule_id": rule, "message": "technical contract failed"}])

    def test_settlement_bound_equality_passes(self):
        detail = self.value["profiles"][CANDIDATE][0]["lifecycle_detail"]["cancelled"]
        detail["return_ms"] = detail["bound_ms"]
        self.assertTrue(self.evaluate()["complete"])

    def test_operational_failure_keeps_reliable_findings_and_the_other_profile(self):
        self.fail_text()
        self.value["profiles"][CANDIDATE][2]["process"].update(exit_code=None, timed_out=True)
        scope = self.evaluate()
        self.assertFalse(scope["complete"])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="incomplete", text_order="failed"))
        self.assertEqual(rules(scope, CANDIDATE)["geometry"]["reason"], "incomplete_evidence")
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))
        self.assertTrue(any(error["kind"] == "incomplete" and error["profile"] == CANDIDATE and error.get("process") == 2
                            and error["message"].startswith("bounded child failed") for error in scope["validation_errors"]))

    def test_typed_failure_leaves_sample_rules_incomplete_without_false_overruns(self):
        baseline = copy.deepcopy(self.value)
        for started in (True, False):
            with self.subTest(started=started):
                self.value = copy.deepcopy(baseline)
                row = self.candidate_row()
                row.update(status="DeadlineExceeded", regions=None)
                if not started:
                    row["metrics"].update(detector_runs=0, recognizer_runs=0, mapped_bytes=0,
                                          detector_width=0, detector_height=0, detector_tensor_bytes=0)
                scope = self.evaluate()
                self.assertFalse(scope["complete"])
                self.assertEqual(outcomes(scope, CANDIDATE),
                                 expected(**{rule: "incomplete" for rule in report.SAMPLE_RULES}))
                self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))

    def test_failed_sample_retains_technical_and_numerical_overruns(self):
        row = self.candidate_row()
        row.update(status="DeadlineExceeded", regions=None)
        row["metrics"].update(detector_runs=2, mapped_bytes=2 * 10**9,
                              detector_width=4096, detector_height=4096,
                              detector_tensor_bytes=512 * 1024 * 1024)
        scope = self.evaluate()
        self.assertFalse(scope["complete"])
        changed = {rule: "incomplete" for rule in report.SAMPLE_RULES}
        changed.update(execution_bounds="failed", workload_budgets="failed")
        self.assertEqual(outcomes(scope, CANDIDATE), expected(**changed))
        observed = rules(scope, CANDIDATE)
        self.assertEqual({finding["kind"] for finding in observed["execution_bounds"]["findings"]},
                         {"detector_runs", "mapped_bytes", "detector_tensor_bytes",
                          "detector_width", "detector_height", "tensor_bytes"})
        self.assertEqual([(finding["metric"], finding["observed"])
                          for finding in observed["workload_budgets"]["findings"]],
                         [("mapped_bytes", 2 * 10**9)])
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))

    def test_missing_samples_leave_unrelated_rules_verified(self):
        self.value["profiles"][CANDIDATE][0]["samples"].pop()
        scope = self.evaluate()
        self.assertFalse(scope["complete"])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(**{rule: "incomplete" for rule in report.SAMPLE_RULES}))
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))

    def test_child_that_never_wrote_jsonl_is_incomplete_not_unverifiable(self):
        process = self.value["profiles"][CANDIDATE][2]
        process["process"] = dict(process["process"], exit_code=None, launch_error="synthetic launch failure")
        retain_raw(self.value, self.evidence_root)
        (self.evidence_root / process["raw_path"]).unlink()
        for key in set(process) - {"index", "process", "command", "raw_path", "receipt_path", "receipt_sha256"}:
            del process[key]
        process.update(samples=[], workloads={}, raw_records=[],
                       parse_failure="[Errno 2] No such file or directory: '/host/specific.jsonl'")
        scope = self.evaluate(retain=False)
        self.assertFalse(scope["complete"])
        self.assertEqual({error["kind"] for error in scope["validation_errors"]}, {"incomplete"})
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="incomplete"))
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))

    def test_absent_profile_evidence_is_not_run(self):
        self.value["profiles"][CANDIDATE] = []
        scope = self.evaluate()
        self.assertFalse(scope["complete"])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="not_run"))
        self.assertEqual(rules(scope, CANDIDATE)["text_order"]["reason"], "no_retained_execution")
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))
        self.assertIn({"kind": "incomplete", "profile": CANDIDATE, "message": "0 of 3 fresh processes retained"},
                      scope["validation_errors"])

    def test_preflight_only_report_runs_no_rule(self):
        self.value["profiles"], self.value["execution"] = {}, "preflight-only"
        scope = self.evaluate()
        self.assertFalse(scope["complete"])
        self.assertEqual(outcomes(scope, REFERENCE), expected(default="not_run"))
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="not_run"))
        self.assertIn("'preflight-only' is not a complete run", scope["validation_errors"][0]["message"])

    def test_missing_evidence_root_is_incomplete_not_not_run(self):
        retain_raw(self.value, self.evidence_root)
        self.value["raw_verified"] = True
        scope = report.evaluate(self.value, evidence_root=None, repository_root=self.repository)
        self.assertFalse(scope["complete"])
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="incomplete"))
        self.assertEqual(rules(scope, CANDIDATE)["text_order"]["reason"], "unverified_evidence")
        self.assertEqual(scope["validation_errors"][0]["message"], "original raw JSONL evidence root is required")

    def test_report_sample_edit_cannot_erase_original_failure_or_pass(self):
        row = self.candidate_row()
        self.value["budgets"]["workloads"][row["workload"]]["max_ms"] = 200
        freeze_policies(self.repository, (self.value, self.windows))
        row["metrics"]["elapsed_ms"] = 201
        retain_raw(self.value, self.evidence_root)
        digest = report.sha256(self.evidence_root / self.value["profiles"][CANDIDATE][0]["raw_path"])
        row["metrics"]["elapsed_ms"] = 199
        scope = self.assert_unverified("projection mismatch: samples", profiles=(CANDIDATE,), retain=False)
        budget = rules(scope, CANDIDATE)["workload_budgets"]
        self.assertEqual((budget["outcome"], budget["reason"]), ("incomplete", "unverified_evidence"))
        self.assertIn(201, [finding["observed"] for finding in budget["findings"]])
        self.assertEqual(scope["profiles"][CANDIDATE]["metrics"]["workloads"][row["workload"]]["max_ms"], 201)
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))
        self.assertEqual(report.sha256(self.evidence_root / self.value["profiles"][CANDIDATE][0]["raw_path"]), digest)

    def test_report_edits_cannot_erase_original_observed_failures(self):
        base = copy.deepcopy(self.value)
        name = self.candidate_row()["workload"]

        def growth(process, original):
            process["workloads"][name]["rust_growth_bytes"] = 10**10 if original else 0
            return "workloads", "workload_budgets"

        def settlement(process, original):
            process["lifecycle_detail"]["cancelled"]["worker_joined"] = not original
            return "lifecycle_detail", "lifecycle"

        for mutate in (growth, settlement):
            with self.subTest(mutate=mutate.__name__):
                self.value = copy.deepcopy(base)
                process = self.value["profiles"][CANDIDATE][0]
                mutate(process, True)
                retain_raw(self.value, self.evidence_root)
                field, rule = mutate(process, False)
                scope = self.assert_unverified(f"projection mismatch: {field}", profiles=(CANDIDATE,), retain=False)
                self.assertTrue(rules(scope, CANDIDATE)[rule]["findings"])

    def test_report_cannot_delete_original_failure_records_or_timeouts(self):
        base = copy.deepcopy(self.value)
        for field in ("raw_records", "process"):
            with self.subTest(field=field):
                self.value = copy.deepcopy(base)
                process = self.value["profiles"][CANDIDATE][0]
                if field == "raw_records":
                    process["raw_records"].append({"type": "failure", "stage": "initialization", "status": "VisionFailed"})
                else:
                    process["process"].update(exit_code=124, timed_out=True, cleanup_ok=False)
                retain_raw(self.value, self.evidence_root)
                process["raw_records"].clear()
                process["process"].update(exit_code=0, timed_out=False, cleanup_ok=True)
                scope = self.assert_unverified("mismatch", profiles=(CANDIDATE,), retain=False)
                self.assertTrue(any(error["kind"] == "incomplete" and error.get("process") == 0
                                    for error in scope["validation_errors"]))

    def test_original_process_receipt_is_required_and_digest_bound(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"][CANDIDATE][0]["receipt_sha256"] = "0" * 64
        self.assert_unverified("receipt digest mismatch", profiles=(CANDIDATE,), retain=False)
        (self.evidence_root / self.value["profiles"][CANDIDATE][0]["receipt_path"]).unlink()
        scope = self.assert_unverified("process receipt unavailable", profiles=(CANDIDATE,), retain=False)
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True))

    def test_report_command_cannot_replace_original_invocation(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"][CANDIDATE][0]["command"][-1] = "2"
        self.assert_unverified("projection mismatch: command", profiles=(CANDIDATE,), retain=False)

    def test_unreported_original_attempts_make_the_scope_unverifiable(self):
        retain_raw(self.value, self.evidence_root)
        (self.evidence_root / "failed-before-observation.process.json").write_bytes(
            report.document_bytes({"index": 0, "command": ["missing"], "process": {"launch_error": "missing"}}))
        self.assert_unverified("receipt attempt set differs", retain=False)
        (self.evidence_root / "failed-attempt-never-retried.jsonl").write_text(
            '{"type":"failure","status":"VisionFailed"}\n', encoding="utf-8")
        self.assert_unverified("raw JSONL attempt set differs", retain=False)

    def test_missing_or_mismatched_original_jsonl_cannot_pass(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"][CANDIDATE][0]["raw_sha256"] = "a" * 64
        self.assert_unverified("original raw JSONL digest mismatch", profiles=(CANDIDATE,), retain=False)
        (self.evidence_root / self.value["profiles"][CANDIDATE][0]["raw_path"]).unlink()
        self.assert_unverified("original raw JSONL unavailable", profiles=(CANDIDATE,), retain=False)

    def test_raw_path_cannot_escape_actual_evidence_root(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"][CANDIDATE][0]["raw_path"] = "../outside.jsonl"
        self.assert_unverified("unsafe relative evidence path", profiles=(CANDIDATE,), retain=False)

    def test_extra_profile_cannot_hide_original_failed_attempt(self):
        process = copy.deepcopy(self.value["profiles"][CANDIDATE][0])
        process["profile"] = "hidden-attempt"
        process["raw_records"].append({"type": "failure", "status": "VisionFailed"})
        self.value["profiles"]["hidden-attempt"] = [process]
        self.assert_unverified("undeclared attempt")

    def test_declared_profile_cannot_hide_another_profiles_failed_attempt(self):
        failed = self.value["profiles"][REFERENCE][1]
        failed["process"].update(exit_code=None, timed_out=True)
        retain_raw(self.value, self.evidence_root)
        retry = copy.deepcopy(failed)
        retry["pid"] += 1000
        retry["process"].update(exit_code=0, timed_out=False)
        retain_raw({"profiles": {"retry-reference": [retry]}}, self.evidence_root)
        self.value["profiles"][REFERENCE][1] = retry
        self.value["profiles"][CANDIDATE].append(failed)
        scope = self.evaluate(retain=False)
        self.assertFalse(scope["complete"])
        self.assertEqual(outcomes(scope, REFERENCE), expected(reference=True, default="incomplete"))
        self.assertEqual(outcomes(scope, CANDIDATE), expected(default="incomplete"))

    def test_scope_bindings_and_policies_are_verified(self):
        base = copy.deepcopy(self.value)

        def edit(path, change):
            def mutate(value):
                target = value
                for key in path[:-1]:
                    target = target[key]
                change(target, path[-1])
            return mutate

        def assign(new):
            return lambda target, key: target.__setitem__(key, new)

        def delete(target, key):
            del target[key]

        cases = (
            ("identity/cohort mismatch", edit(("profiles", CANDIDATE, 0, "binding", "cpu"), assign("another CPU"))),
            ("runtime_sha256", edit(("binding", "runtime_sha256"), delete)),
            ("wrong release target", edit(("binding", "target"), assign("x86_64-unknown-linux-gnu"))),
            ("host cohort absent from frozen release allowlist", edit(("binding", "os"), assign("macOS 27.0.1"))),
            ("missing precommitted absolute budgets", edit(("budgets",), assign(None))),
            ("embedded budgets differs from frozen input bytes",
             edit(("budgets", "workloads", self.candidate_row()["workload"], "max_ms"), assign(2 * 10**9))),
            ("embedded host_policy differs from frozen input bytes",
             edit(("host_policy", "approved_cohorts"), lambda target, key: target[key].append({"cohort": {}, "approval": "edited"}))),
            ("compiled descriptor authority", edit(("identities", CANDIDATE, "execution_sha256"), assign("a" * 64))),
            ("diagnostic execution", edit(("diagnostic",), assign(True))),
            ("repository-relative policy path", edit(("policy_inputs", "budgets", "path"), delete)),
            ("historical Git tree/blob authority",
             lambda value: value["policy_inputs"]["budgets"].update(path=value["policy_inputs"]["host_policy"]["path"])),
            ("source tree differs from Git commit authority",
             lambda value: value["binding"].update(source_tree="a" * 40, build_source_tree="a" * 40)),
            ("compiled build provenance mismatch: build_compiler", edit(("binding", "compiler"), assign("new selected compiler"))),
            ("compiled build source is dirty or unavailable", edit(("binding", "build_source_dirty"), assign(True))),
            ("invalid binding build_source_revision", edit(("binding", "build_source_revision"), assign("unavailable"))),
            ("compiled build compiler is unavailable",
             lambda value: value["binding"].update(compiler="unavailable", build_compiler="unavailable")),
            ("uncommitted implementation source", edit(("source_changes",), assign("?? implementation.rs"))),
            ("preflight/operational failure", edit(("preflight_failures",), assign(["/host/private failure"]))),
            ("unsupported report schema version", edit(("schema_version",), assign(3))),
        )
        for fragment, mutate in cases:
            with self.subTest(fragment=fragment):
                self.value = copy.deepcopy(base)
                mutate(self.value)
                profiles = (CANDIDATE,) if fragment == "identity/cohort mismatch" else (REFERENCE, CANDIDATE)
                scope = self.assert_unverified(fragment, profiles=profiles)
                self.assertNotIn("/host/private", json.dumps(scope))

    def test_precommitted_host_policy_can_approve_a_new_cohort(self):
        os_name = "macOS 27.0.1"
        self.value["binding"]["os"] = os_name
        self.value["budgets"]["cohort"]["os"] = os_name
        self.value["host_policy"]["approved_cohorts"][0]["cohort"]["os"] = os_name
        freeze_policies(self.repository, (self.value, self.windows))
        self.assertTrue(self.evaluate()["complete"])

    def test_rehashing_embedded_policies_cannot_forge_precommit(self):
        base = copy.deepcopy(self.value)
        for key in ("budgets", "host_policy"):
            with self.subTest(policy=key):
                self.value = copy.deepcopy(base)
                if key == "budgets":
                    self.value["budgets"]["workloads"][self.candidate_row()["workload"]]["max_ms"] *= 2
                else:
                    self.value["host_policy"]["approved_cohorts"].append({"cohort": {}, "approval": "forged"})
                evidence = self.value["policy_inputs"][key]
                forged = policy_evidence(policy_documents(self.value)[key], evidence["revision"], evidence["path"])
                self.value["policy_inputs"][key] = forged
                self.value["budget_sha256" if key == "budgets" else "host_policy_sha256"] = forged["sha256"]
                (self.repository / evidence["path"]).write_bytes(base64.b64decode(forged["base64"]))
                self.assert_unverified(f"frozen {key} differs from historical Git tree/blob authority")
                git(self.repository, "checkout", "--", evidence["path"])

    def test_missing_git_objects_never_fall_back_to_embedded_bytes(self):
        retain_raw(self.value, self.evidence_root)
        scope = report.evaluate(self.value, evidence_root=self.evidence_root, repository_root=self.root)
        self.assertFalse(scope["complete"])
        self.assertTrue(any("Git authority" in error["message"] for error in scope["validation_errors"]))
        blob = self.value["policy_inputs"]["budgets"]["git_blob"]
        (self.repository / ".git" / "objects" / blob[:2] / blob[2:]).unlink()
        self.assert_unverified("frozen budgets Git authority unavailable")

    def test_policy_committed_after_implementation_is_not_precommitted(self):
        (self.repository / "later-policy-marker").write_text("later\n", encoding="utf-8")
        git(self.repository, "add", ".")
        git(self.repository, "-c", "user.name=Synthetic Evidence", "-c", "user.email=synthetic@example.invalid",
            "-c", "commit.gpgsign=false", "commit", "-qm", "later policy commit")
        later = git(self.repository, "rev-parse", "HEAD")
        self.value["binding"]["frozen_revision"] = later
        self.value["budgets"]["frozen_revision"] = later
        for evidence in self.value["policy_inputs"].values():
            evidence["revision"] = later
        for processes in self.value["profiles"].values():
            for process in processes:
                process["binding"] = copy.deepcopy(self.value["binding"])
        self.assert_unverified("merge-base --is-ancestor")

    def test_old_build_cannot_claim_new_head_when_catalog_is_unchanged(self):
        (self.repository / "implementation.rs").write_text("// synthetic implementation B\n", encoding="utf-8")
        git(self.repository, "add", ".")
        git(self.repository, "-c", "user.name=Synthetic Evidence", "-c", "user.email=synthetic@example.invalid",
            "-c", "commit.gpgsign=false", "commit", "-qm", "new implementation, unchanged catalog")
        revision, tree = git(self.repository, "rev-parse", "HEAD"), git(self.repository, "rev-parse", "HEAD^{tree}")
        self.value["binding"].update(source_revision=revision, source_tree=tree)
        for processes in self.value["profiles"].values():
            for process in processes:
                process["binding"] = copy.deepcopy(self.value["binding"])
        self.assert_unverified("compiled build provenance mismatch: build_source_revision")

    def test_historical_accepted_field_is_identity_data_not_policy(self):
        baseline = outcomes(self.evaluate(), CANDIDATE)
        for index, accepted in enumerate(((True, False), (False, False), (False, True))):
            with self.subTest(accepted=accepted):
                self.value = synthetic_report(schema_version=1, accepted=accepted)
                freeze_policies(self.repository, (self.value,))
                self.evidence_root = self.root / f"historical-{index}"
                scope = self.evaluate()
                self.assertEqual(scope["validation_errors"], [])
                self.assertEqual(outcomes(scope, CANDIDATE), baseline)
                self.assertEqual(scope["evidence"]["report_schema_version"], 1)
                self.assertNotIn("accepted", set(walk(scope)))

    def test_current_schema_rejects_a_historical_identity_shape(self):
        for identity in [self.value["identities"][CANDIDATE]] + [
                process["identity"] for process in self.value["profiles"][CANDIDATE]]:
            identity["accepted"] = False
        for command in self.value["commands"]:
            description = json.loads(command["process"]["stdout"])
            if description["identity"]["profile_id"] == CANDIDATE:
                description["identity"]["accepted"] = False
                command["process"]["stdout"] = json.dumps(description)
        self.assert_unverified("identity fields differ from the report schema", profiles=(CANDIDATE,))

    def test_single_target_bundle_shows_other_target_not_run(self):
        path = self.write_report(self.value, self.evidence_root)
        bundle = report.evaluate_reports([path], evidence_roots=[self.evidence_root], repository_root=self.repository)
        self.assertEqual(bundle["schema_version"], report.EVALUATION_SCHEMA_VERSION)
        self.assertEqual(bundle["evaluator_revision"], "sha256:" + report.sha256(Path(report.__file__)))
        self.assertTrue(bundle["complete"])
        self.assertEqual(bundle["not_run_targets"], [WINDOWS])
        self.assertEqual(bundle["evaluations"][0]["evidence"]["report_sha256"], report.sha256(path))
        with self.assertRaises(ValueError):
            report.evaluate_reports([path], evidence_roots=[], repository_root=self.repository)

    def test_incomplete_scope_does_not_invalidate_another_scope(self):
        self.fail_text()
        apple = self.write_report(self.value, self.evidence_root)
        windows = self.write_report(self.windows, self.windows_root)
        alone = report.evaluate_reports([apple], evidence_roots=[self.evidence_root], repository_root=self.repository)
        (self.windows_root / self.windows["profiles"][CANDIDATE][0]["raw_path"]).unlink()
        bundle = report.evaluate_reports([apple, windows], evidence_roots=[self.evidence_root, self.windows_root],
                                         repository_root=self.repository)
        self.assertFalse(bundle["complete"])
        self.assertEqual(bundle["not_run_targets"], [])
        self.assertEqual(bundle["evaluations"][0], alone["evaluations"][0])
        self.assertEqual(outcomes(bundle["evaluations"][0], CANDIDATE), expected(text_order="failed"))
        windows_scope = bundle["evaluations"][1]
        self.assertFalse(windows_scope["complete"])
        self.assertNotIn("passed", outcomes(windows_scope, CANDIDATE).values())
        self.assertEqual(outcomes(windows_scope, REFERENCE), expected(reference=True))

    def test_unattributed_report_does_not_claim_an_unrun_target(self):
        apple = self.write_report(self.value, self.evidence_root)
        broken = self.root / "unreadable-report.json"
        broken.write_bytes(b'{"binding":')
        bundle = report.evaluate_reports([apple, broken], evidence_roots=[self.evidence_root, self.windows_root],
                                         repository_root=self.repository)
        self.assertFalse(bundle["complete"])
        self.assertIsNone(bundle["not_run_targets"])
        self.assertEqual(outcomes(bundle["evaluations"][0], CANDIDATE), expected())
        self.assertIsNone(bundle["evaluations"][1]["scope"]["target"])
        self.assertEqual({error["kind"] for error in bundle["evaluations"][1]["validation_errors"]},
                         {"integrity"})

    def test_malformed_report_cli_preserves_other_target_and_json_output(self):
        apple = self.write_report(self.value, self.evidence_root)
        windows = self.write_report(self.windows, self.windows_root)
        payloads = {
            "overflow": b'{"binding":{"target":"' + WINDOWS.encode() +
                        b'"},"process_limits":{"timeout_seconds":1e999}}',
            "container_target": report.document_bytes({"binding": {"target": [WINDOWS]}}),
            "unsupported_target": report.document_bytes({"binding": {"target": "unsupported"}}),
            "deep_report": b"[" * 2000 + b"0" + b"]" * 2000,
            "surrogate_provenance": json.dumps({"binding": {"target": WINDOWS, "cpu": chr(0xD800)}}).encode(),
            "oversized_integer": report.document_bytes({"binding": {"target": WINDOWS},
                                                       "process_limits": {"timeout_seconds": 10**400}}),
            "structured_provenance": report.document_bytes({
                "binding": {"target": WINDOWS, "cpu": {"unexpected": "field"}},
                "process_limits": {"timeout_seconds": {"unexpected": "field"}}}),
        }
        for kind, payload in payloads.items():
            with self.subTest(kind=kind):
                windows.write_bytes(payload)
                result = self.run_cli("evaluate", apple, windows, "--repository", self.repository)
                self.assertEqual(result.returncode, 2, result.stderr.decode("utf-8"))
                bundle = json.loads(result.stdout.decode("utf-8"))
                self.assertFalse(bundle["complete"])
                self.assertTrue(bundle["evaluations"][0]["complete"])
                self.assertEqual(outcomes(bundle["evaluations"][0], CANDIDATE), expected())
                invalid = bundle["evaluations"][1]
                self.assertEqual({error["kind"] for error in invalid["validation_errors"]}, {"integrity"})
                self.assertIsNone(invalid["scope"]["cpu"])
                if kind in ("overflow", "container_target", "unsupported_target", "deep_report", "surrogate_provenance"):
                    self.assertIsNone(invalid["scope"]["target"])
                    self.assertIsNone(bundle["not_run_targets"])
                elif kind == "structured_provenance":
                    self.assertIsNone(invalid["evidence"]["process_limits"]["timeout_seconds"])
        self.windows["profiles"][CANDIDATE][0]["samples"][2]["metrics"].update(
            detector_width=1e308, detector_height=1e308)
        self.write_report(self.windows, self.windows_root)
        result = self.run_cli("evaluate", apple, windows, "--repository", self.repository)
        self.assertEqual(result.returncode, 2, result.stderr.decode("utf-8"))
        bundle = json.loads(result.stdout.decode("utf-8"))
        self.assertEqual(outcomes(bundle["evaluations"][0], CANDIDATE), expected())
        self.assertEqual(outcomes(bundle["evaluations"][1], CANDIDATE)["execution_bounds"], "failed")

    def test_invalid_raw_metrics_and_oracles_preserve_other_target(self):
        apple = self.write_report(self.value, self.evidence_root)
        for kind in ("growth_integer", "oracle_overflow", "deep_raw"):
            with self.subTest(kind=kind):
                value = copy.deepcopy(self.windows)
                if kind == "growth_integer":
                    workload = next(iter(value["profiles"][CANDIDATE][0]["workloads"].values()))
                    workload["rust_growth_bytes"] = 10**400
                elif kind == "oracle_overflow":
                    value["fixtures"][0]["regions"][0]["source_relative_quad"] = [[1.7e308, 1.7e308]] * 4
                windows = self.write_report(value, self.windows_root)
                if kind == "deep_raw":
                    process = value["profiles"][CANDIDATE][0]
                    raw = self.windows_root / process["raw_path"]
                    raw.write_bytes(raw.read_bytes() + b"[" * 2000 + b"0" + b"]" * 2000 + b"\n")
                    process["raw_sha256"] = report.sha256(raw)
                    windows.write_bytes(report.document_bytes(value))
                result = self.run_cli("evaluate", apple, windows, "--repository", self.repository)
                self.assertEqual(result.returncode, 2, result.stderr.decode("utf-8"))
                bundle = json.loads(result.stdout.decode("utf-8"))
                self.assertTrue(bundle["evaluations"][0]["complete"])
                self.assertEqual(outcomes(bundle["evaluations"][0], CANDIDATE), expected())
                invalid = bundle["evaluations"][1]
                self.assertFalse(invalid["complete"])
                if kind == "growth_integer":
                    self.assertEqual(outcomes(invalid, CANDIDATE), expected(workload_budgets="incomplete"))
                    self.assertEqual(outcomes(invalid, REFERENCE), expected(reference=True))
                else:
                    self.assertNotIn("passed", outcomes(invalid, CANDIDATE).values())

    def test_cli_quality_failure_exits_zero_and_invalid_evidence_exits_nonzero(self):
        self.fail_text()
        apple = self.write_report(self.value, self.evidence_root)
        windows = self.write_report(self.windows, self.windows_root)
        result = self.run_cli("evaluate", apple, windows, "--repository", self.repository)
        self.assertEqual(result.returncode, 0, result.stderr)
        bundle = json.loads(result.stdout.decode("utf-8"))
        self.assertTrue(bundle["complete"])
        self.assertEqual(outcomes(bundle["evaluations"][0], CANDIDATE), expected(text_order="failed"))
        self.assertEqual(outcomes(bundle["evaluations"][1], CANDIDATE), expected())
        self.windows["profiles"][CANDIDATE][0]["samples"][2]["metrics"]["elapsed_ms"] = 1
        windows.write_bytes(report.document_bytes(self.windows))
        result = self.run_cli("evaluate", apple, windows, "--evidence-roots", self.evidence_root, self.windows_root,
                              "--repository", self.repository)
        self.assertEqual(result.returncode, 2, result.stderr)
        bundle = json.loads(result.stdout.decode("utf-8"))
        self.assertFalse(bundle["complete"])
        self.assertTrue(bundle["evaluations"][0]["complete"])
        self.assertTrue(any("projection mismatch: samples" in error["message"]
                            for error in bundle["evaluations"][1]["validation_errors"]))

    def test_cli_has_only_run_and_evaluate_commands(self):
        path = self.write_report(self.value, self.evidence_root)
        for command in ("validate", "admission"):
            with self.subTest(command=command):
                result = self.run_cli(command, path)
                self.assertEqual(result.returncode, 2)
                self.assertIn(b"invalid choice", result.stderr)
        result = self.run_cli("evaluate", path, path, "--evidence-roots", self.evidence_root)
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"one directory per report", result.stderr)

    def test_evaluation_output_is_publishable(self):
        path = self.write_report(self.value, self.evidence_root)
        private = (str(self.root), str(self.root.resolve()), self.value["binding"]["host_id"],
                   "/synthetic/private/python", "--model-evaluation", "/synthetic/licensed")
        bundle = report.evaluate_reports([path], evidence_roots=[self.evidence_root], repository_root=self.repository)
        self.assertTrue(bundle["complete"])
        broken = report.evaluate_reports([path, self.root / "missing.json"],
                                         evidence_roots=[self.root / "missing", self.evidence_root],
                                         repository_root=self.root / "missing-repository")
        self.assertFalse(broken["complete"])
        for value in (bundle, broken):
            text = report.document_bytes(value).decode("utf-8")
            for fragment in private:
                self.assertNotIn(fragment, text)
        scope = bundle["evaluations"][0]
        self.assertEqual(scope["scope"]["cohort_id"], self.value["binding"]["cohort_id"])
        self.assertEqual(scope["evidence"]["policies"]["budgets"]["path"], self.value["policy_inputs"]["budgets"]["path"])
        self.assertEqual(scope["profiles"][CANDIDATE]["processes"][0]["raw_sha256"],
                         self.value["profiles"][CANDIDATE][0]["raw_sha256"])

    def test_geometry_iou_equality_has_no_stricter_hidden_gate(self):
        source = {key: "synthetic" for key in ("stamp", "transform", "effective_region", "backend", "output_space", "effective_zones")}
        expected_regions = [{"text_nfc": "A", "source_relative_quad": [[.25, .25], [.5, .25], [.5, .5], [.25, .5]]}]
        observation = {"status": "Ok", "source_correlation": True, "source": source, "result_source": dict(source),
                       "regions": [{"text": "A", "confidence": 0, "quad": [[.125, .25], [.625, .25], [.625, .5], [.125, .5]]}], "groups": None}
        self.assertEqual(report.quality(observation, expected_regions, "singular"), [])


class QualityTests(unittest.TestCase):
    def setUp(self):
        source = {key: "synthetic" for key in
                  ("stamp", "transform", "effective_region", "backend", "output_space", "effective_zones")}
        quad = [[.25, .25], [.5, .25], [.5, .5], [.25, .5]]
        self.expected = [{"text_nfc": text, "source_relative_quad": copy.deepcopy(quad)}
                         for text in ("次の攻撃が25%", "残り50%")]
        self.observation = {"status": "Ok", "source_correlation": True, "source": source,
                            "result_source": dict(source), "groups": [[0, 1], [0, 1]],
                            "regions": [{"text": region["text_nfc"], "confidence": .1,
                                         "quad": copy.deepcopy(quad)} for region in self.expected]}

    def test_invalid_geometry_preserves_text_and_unmatched_findings(self):
        self.observation["regions"][0]["text"] = "WRONG"
        self.observation["regions"][0]["quad"][0][0] = 1.0000001
        self.observation["regions"].append({"text": "noise", "confidence": .9, "quad": None})
        findings = report.quality(self.observation, self.expected, "singular")
        self.assertEqual({(finding["rule_id"], finding["kind"], finding.get("region")) for finding in findings},
                         {("region_count", "count", None), ("geometry", "invalid_quad", 0),
                          ("geometry", "invalid_quad", 2), ("text_order", "text", 0),
                          ("unexpected_detection", "unmatched", 2)})

    def test_percent_width_equivalence_in_both_directions_preserves_raw_text(self):
        for actual, expected in (("次の攻撃が25％", "次の攻撃が25%"),
                                 ("次の攻撃が25%", "次の攻撃が25％")):
            with self.subTest(actual=actual, expected=expected):
                self.observation["regions"][0]["text"] = actual
                self.expected[0]["text_nfc"] = expected
                before = copy.deepcopy((self.observation, self.expected))
                self.assertEqual(report.quality(self.observation, self.expected, "grouped"), [])
                self.assertEqual((self.observation, self.expected), before)

    def test_nfc_and_trim_apply_to_both_texts(self):
        for actual, expected in ((" \tか\u309925％\n", "が25%"),
                                 ("が25%", " \tか\u309925％\n")):
            with self.subTest(actual=actual, expected=expected):
                self.observation["regions"][0]["text"] = actual
                self.expected[0]["text_nfc"] = expected
                self.assertEqual(report.quality(self.observation, self.expected, "grouped"), [])

    def test_other_character_differences_remain_text_failures(self):
        for actual, expected in (("２５％", "25%"), ("Ａ25％", "A25%"),
                                 ("ｶ25％", "カ25%"), ("25‰", "25%"),
                                 ("26％", "25%"), ("25 ％", "25%")):
            with self.subTest(actual=actual, expected=expected):
                self.observation["regions"][0]["text"] = actual
                self.expected[0]["text_nfc"] = expected
                self.assertEqual(report.quality(self.observation, self.expected, "grouped"),
                                 [{"rule_id": "text_order", "kind": "text", "region": 0,
                                   "expected": expected, "observed": actual}])

    def test_percent_equivalence_preserves_nontext_and_order_rules(self):
        baseline = copy.deepcopy(self.observation)
        for change, rule in (("order", "text_order"), ("geometry", "geometry"), ("confidence", "confidence"),
                             ("groups", "text_order"), ("count", "region_count"), ("source", "source_correlation")):
            with self.subTest(change=change):
                observation = copy.deepcopy(baseline)
                observation["regions"][0]["text"] = "次の攻撃が25％"
                if change == "order":
                    observation["regions"].reverse()
                elif change == "geometry":
                    observation["regions"][0]["quad"] = [[.6, .6], [.8, .6], [.8, .8], [.6, .8]]
                elif change == "confidence":
                    observation["regions"][0]["confidence"] = 1.01
                elif change == "groups":
                    observation["groups"][0].reverse()
                elif change == "count":
                    observation["regions"].pop()
                    observation["groups"] = [[0], [0]]
                else:
                    observation["result_source"]["stamp"] = "another-frame"
                self.assertEqual({finding["rule_id"] for finding in report.quality(observation, self.expected, "grouped")},
                                 {rule})


class StabilityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.original = synthetic_report()

    def setUp(self):
        self.value = copy.deepcopy(self.original)
        self.profile = REFERENCE
        self.processes = self.value["profiles"][self.profile]

    def evidence(self):
        return report.evaluate_profile(self.processes, self.profile, self.value["binding"],
                                       self.value["fixtures"], self.value["budgets"],
                                       self.value["identities"][self.profile],
                                       self.value["budgets"]["process_limits"]["lifecycle_bound_ms"])

    def assert_only(self, evidence, rules_with_findings):
        self.assertEqual((evidence.gaps, evidence.invalid), ([], []))
        self.assertEqual({rule for rule, findings in evidence.findings.items() if findings}, rules_with_findings)

    def test_repeated_equivalent_widths_are_stable_without_rewriting_observations(self):
        changed = 0
        for process in self.processes:
            for index, row in enumerate(process["samples"]):
                if index % 2:
                    for region in row["regions"]:
                        if "%" in region["text"]:
                            region["text"] = region["text"].replace("%", "％")
                            changed += 1
        self.assertGreater(changed, 0)
        before = copy.deepcopy(self.processes)
        self.assert_only(self.evidence(), set())
        self.assertEqual(self.processes, before)

    def test_valid_geometry_and_confidence_changes_still_break_stability(self):
        self.assert_only(self.evidence(), set())
        baseline = copy.deepcopy(self.processes)
        for change in ("geometry", "confidence"):
            with self.subTest(change=change):
                self.processes = copy.deepcopy(baseline)
                row = self.processes[0]["samples"][2]
                fixture = next(fixture for fixture in self.value["fixtures"]
                               if row["workload"] == f"{fixture['id']}/singular")
                region = row["regions"][0]
                if change == "geometry":
                    region["quad"][0][0] += -.000001 if region["quad"][0][0] > .5 else .000001
                else:
                    region["confidence"] = .2
                self.assertEqual(report.quality(row, fixture["regions"], "singular"), [])
                self.assert_only(self.evidence(), {"stability"})


class Utf8ReadsTests(unittest.TestCase):
    def test_corpus_uses_utf8_under_cp1252_default_decoding(self):
        expected, identities = report.frozen_corpus(), report.corpus_identities()
        original_read_text = Path.read_text

        def cp1252_read_text(path, encoding=None, errors=None):
            return original_read_text(path, encoding=encoding or "cp1252", errors=errors)

        with tempfile.TemporaryDirectory(prefix="ocr-report-encoding-") as directory:
            control = Path(directory) / "utf8.txt"
            control.write_text("日本語", encoding="utf-8")
            with mock.patch.object(Path, "read_text", cp1252_read_text):
                self.assertNotEqual(control.read_text(), "日本語")
                self.assertEqual(report.frozen_corpus(), expected)
                self.assertEqual(report.corpus_identities(), identities)
        self.assertTrue(any(any(ord(character) > 127 for character in region["text_nfc"])
                            for row in expected for region in row["regions"]))


if __name__ == "__main__":
    unittest.main()
