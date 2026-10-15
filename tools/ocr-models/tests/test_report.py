"""Synthetic protocol/classifier tests only: these are never measurements or host budgets."""
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
        "-c", "commit.gpgsign=false", "commit", "-qm", "freeze synthetic policy inputs")
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


def synthetic_report(target="aarch64-apple-darwin"):
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
                    "target": target, "os": "macOS 15.0" if target.startswith("aarch64") else "Windows-11-synthetic",
                    "provider": "cpu", "runtime": "1.29.0-api17", "intra_op": 1, "inter_op": 1,
                    "qualified_host": True, "host_approval": "synthetic-test-only", "serial_execution": True,
                    "frozen_revision": "3" * 40})
    binding.update(build_source_revision=binding["source_revision"], build_source_tree=binding["source_tree"],
                   build_compiler=binding["compiler"], build_source_dirty=False)
    limits = {"timeout_seconds": 1000, "output_limit_bytes": 100000000, "cleanup_seconds": 5, "lifecycle_bound_ms": 5000}
    value = {"schema_version": 1, "criteria_id": report.CRITERIA["criteria_id"], "binding": binding,
             "fixtures": fixtures, "corpora": report.corpus_identities(),
             "reference_profile": "synthetic-reference", "candidate_profile": "synthetic-candidate",
             "identities": {}, "profiles": {}, "commands": [], "preflight_failures": [],
             "diagnostic": False, "process_limits": limits}
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
    for profile, accepted, elapsed in (("synthetic-reference", True, 100), ("synthetic-candidate", False, 80)):
        identity = {"profile_id": profile, "model_id": profile, "model_version": "synthetic-v1",
                    "preprocessing_id": "synthetic", "decoder_id": "synthetic", "normalization_id": "synthetic",
                    "execution_sha256": "5" * 64, "accepted": accepted,
                    "limits": {"detector_width": 736, "detector_height": 736, "tensor_bytes": 256 * 1024 * 1024},
                    "resources": [{"kind": kind, "relative_path": kind + ".onnx", "bytes": 10, "sha256": "6" * 64}
                                  for kind in ("detector", "recognizer")]}
        value["identities"][profile] = identity
        descriptor_process = dict(good_process, stdout=json.dumps({"identity": identity, "build": build}))
        value["commands"].append({"command": ["synthetic-onnx-cpu", "--model-admission", "--describe-profile", "--profile", profile], "process": descriptor_process})
        processes = []
        for index in range(3):
            process = {"index": index, "pid": 100 + index + (0 if accepted else 10), "binding": copy.deepcopy(binding),
                       "profile": profile, "identity": copy.deepcopy(identity), "build": copy.deepcopy(build),
                       "target": target, "opencv": binding["opencv"], "runtime": binding["runtime"], "provider": "cpu",
                       "vocabulary_supported": True, "process": dict(good_process),
                       "command": ["synthetic-onnx-cpu", "--model-admission", "--profile", profile, "--process-index", str(index)],
                       "metrics": {"cold_open_ms": 1, "close_ms": 1, "peak_resident_bytes": 100, "model_bytes": 20, "auxiliary_bytes": 0},
                       "lifecycle": dict(report.CRITERIA["required_lifecycle"], initialization_cancelled="Cancelled"),
                       "samples": [], "workloads": {}, "close_fault": None, "raw_records": []}
            process["collector_scope"] = {"rust_accounting": "SDK plus raw observation construction/serialization",
                                          "native_peak_rss": "entire fresh process including cold initialization, workloads and lifecycle"}
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


class ClassifierTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.original = synthetic_report()
        cls.original_windows = synthetic_report("x86_64-pc-windows-msvc")

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ocr-report-integrity-")
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

    def verdict(self, *, retain=True):
        if retain:
            retain_raw(self.value, self.evidence_root)
        return report.classify(self.value, evidence_root=self.evidence_root, repository_root=self.repository)

    def admit(self, values, *, retain=True):
        roots = [self.windows_root if value["binding"]["target"].startswith("x86_64") else self.evidence_root for value in values]
        if retain:
            for value, root in zip(values, roots):
                retain_raw(value, root)
        return report.admission(values, evidence_roots=roots, repository_root=self.repository)

    def candidate_row(self):
        return self.value["profiles"]["synthetic-candidate"][0]["samples"][2]

    def assert_refused(self, fragment, *, retain=True):
        verdict = self.verdict(retain=retain)
        self.assertFalse(verdict["target_qualified"])
        self.assertIsNone(verdict["candidate"])
        self.assertTrue(any(fragment in failure for failure in verdict["failures"]), verdict["failures"][:10])

    def test_exact_eighty_percent_is_fast_but_one_target_does_not_admit(self):
        verdict = self.verdict()
        self.assertEqual(verdict["failures"], [])
        self.assertTrue(verdict["target_qualified"])
        self.assertEqual(verdict["candidate"], "fast")
        self.assertFalse(verdict["admitted"])
        self.assertFalse(self.admit([self.value])["admitted"])

    def test_one_slow_workload_is_balanced_not_averaged_fast(self):
        name = self.candidate_row()["workload"]
        for process in self.value["profiles"]["synthetic-candidate"]:
            for row in process["samples"]:
                if row["workload"] == name:
                    row["metrics"]["elapsed_ms"] = 81
        self.assertEqual(self.verdict()["candidate"], "balanced")

    def test_incomplete_samples_fail(self):
        self.value["profiles"]["synthetic-candidate"][0]["samples"].pop()
        self.assert_refused("sample schedule")

    def test_missing_fresh_process_fails(self):
        self.value["profiles"]["synthetic-candidate"].pop()
        self.assert_refused("three fresh")

    def test_wrong_identity_and_cohort_fail(self):
        self.value["profiles"]["synthetic-candidate"][0]["binding"]["cpu"] = "another CPU"
        self.assert_refused("identity/cohort")

    def test_missing_identity_fails(self):
        del self.value["binding"]["runtime_sha256"]
        self.assert_refused("runtime_sha256")

    def test_wrong_target_fails(self):
        self.value["binding"]["target"] = "x86_64-unknown-linux-gnu"
        self.assert_refused("wrong release target")

    def test_macos27_never_becomes_qualified_by_boolean(self):
        self.value["binding"]["os"] = "macOS 27.0.0"
        self.assert_refused("macOS 27")

    def test_quality_failure_is_not_hidden_by_speed(self):
        self.candidate_row()["regions"][0]["text"] = "wrong"
        self.assert_refused("text/order")

    def test_missing_stage_metric_fails(self):
        del self.candidate_row()["metrics"]["recognizer_decode_ms"]
        self.assert_refused("missing sample metric")

    def test_missing_native_rss_fails(self):
        self.value["profiles"]["synthetic-candidate"][0]["metrics"]["peak_resident_bytes"] = None
        self.assert_refused("peak_resident_bytes")

    def test_one_hard_budget_failure_is_not_aggregated_away(self):
        self.candidate_row()["metrics"]["elapsed_ms"] = 10**10
        self.assert_refused("absolute budget")

    def test_below_threshold_observation_is_kept_and_valid_when_matched(self):
        self.assertEqual(self.candidate_row()["regions"][0]["confidence"], .1)
        self.assertTrue(self.verdict()["target_qualified"])

    def test_unmatched_high_confidence_detection_fails(self):
        self.candidate_row()["regions"].append({"text": "extra", "confidence": .5,
                                               "quad": [[.1, .1], [.2, .1], [.2, .2], [.1, .2]]})
        self.assert_refused("unmatched detection >=0.5")

    def test_nan_confidence_and_instability_fail(self):
        self.candidate_row()["regions"][0]["confidence"] = float("nan")
        self.assert_refused("nonfinite")

    def test_raw_correlation_overrides_handwritten_pass(self):
        self.candidate_row()["result_source"]["stamp"] = "another-frame"
        self.assert_refused("raw source/result")

    def test_missing_absolute_budgets_refuses(self):
        self.value["budgets"] = None
        self.assert_refused("missing precommitted absolute budgets")

    def test_editing_embedded_budget_cap_does_not_edit_frozen_policy(self):
        name = self.candidate_row()["workload"]
        self.value["budgets"]["workloads"][name]["max_ms"] *= 2
        self.assert_refused("embedded budgets differs from frozen input bytes")

    def test_editing_embedded_host_allowlist_does_not_edit_frozen_policy(self):
        self.value["host_policy"]["approved_cohorts"].append({"cohort": {}, "approval": "edited"})
        self.assert_refused("embedded host_policy differs from frozen input bytes")

    def test_editing_execution_digest_does_not_change_compiled_authority(self):
        self.value["identities"]["synthetic-candidate"]["execution_sha256"] = "a" * 64
        self.assert_refused("compiled descriptor authority")

    def test_missing_admitted_deadline_collector_fails(self):
        del self.value["profiles"]["synthetic-candidate"][0]["lifecycle_detail"]["deadline"]
        self.assert_refused("admitted deadline")

    def test_settlement_bound_equality_passes_but_excess_fails(self):
        detail = self.value["profiles"]["synthetic-candidate"][0]["lifecycle_detail"]["cancelled"]
        detail["return_ms"] = detail["bound_ms"]
        self.assertTrue(self.verdict()["target_qualified"])
        detail["return_ms"] += .001
        self.assert_refused("admitted cancelled")

    def test_handwritten_verdict_is_ignored(self):
        self.value["verdict"] = {"candidate": "fast", "admitted": True}
        self.candidate_row()["status"] = "VisionFailed"
        self.assert_refused("status is not Ok")

    def test_diagnostic_never_rates(self):
        self.value["diagnostic"] = True
        self.assert_refused("diagnostic execution")

    def test_unrated_never_admits_new_candidate(self):
        self.assertEqual(report.unrated(already_accepted=False), {"admitted": False, "code": None})
        self.assertEqual(report.unrated(already_accepted=True), {"admitted": False, "code": "unrated"})

    def test_complete_two_target_pair_only_is_eligible(self):
        result = self.admit([self.value, self.windows])
        self.assertEqual(result["failures"], [])
        self.assertTrue(result["admitted"])
        self.assertFalse(self.admit([self.value, copy.deepcopy(self.value)])["admitted"])

    def test_report_sample_edit_cannot_erase_original_201ms_failure(self):
        row = self.candidate_row()
        self.value["budgets"]["workloads"][row["workload"]]["max_ms"] = 200
        freeze_policies(self.repository, (self.value, self.windows))
        row["metrics"]["elapsed_ms"] = 201
        original = self.verdict()
        self.assertFalse(original["target_qualified"])
        digest = report.sha256(self.evidence_root / self.value["profiles"]["synthetic-candidate"][0]["raw_path"])
        row["metrics"]["elapsed_ms"] = 199
        verdict = self.verdict(retain=False)
        self.assertFalse(verdict["target_qualified"])
        self.assertIsNone(verdict["candidate"])
        self.assertTrue(any("projection mismatch: samples" in failure for failure in verdict["failures"]))
        self.assertTrue(any("absolute budget max_ms" in failure for failure in verdict["failures"]))
        self.assertEqual(verdict["summaries"]["synthetic-candidate"][row["workload"]]["max_ms"], 201)
        self.assertEqual(report.sha256(self.evidence_root / self.value["profiles"]["synthetic-candidate"][0]["raw_path"]), digest)

    def test_report_workload_edit_cannot_erase_original_allocation_failure(self):
        process = self.value["profiles"]["synthetic-candidate"][0]
        name = self.candidate_row()["workload"]
        process["workloads"][name]["rust_growth_bytes"] = 10**10
        retain_raw(self.value, self.evidence_root)
        process["workloads"][name]["rust_growth_bytes"] = 0
        verdict = self.verdict(retain=False)
        self.assertFalse(verdict["target_qualified"])
        self.assertTrue(any("projection mismatch: workloads" in failure for failure in verdict["failures"]))
        self.assertTrue(any("absolute budget rust_growth_bytes" in failure for failure in verdict["failures"]))

    def test_report_lifecycle_edit_cannot_erase_original_cleanup_failure(self):
        process = self.value["profiles"]["synthetic-candidate"][0]
        process["lifecycle_detail"]["cancelled"]["worker_joined"] = False
        retain_raw(self.value, self.evidence_root)
        process["lifecycle_detail"]["cancelled"]["worker_joined"] = True
        verdict = self.verdict(retain=False)
        self.assertFalse(verdict["target_qualified"])
        self.assertTrue(any("projection mismatch: lifecycle_detail" in failure for failure in verdict["failures"]))
        self.assertTrue(any("admitted cancelled" in failure for failure in verdict["failures"]))

    def test_report_deleting_failure_records_does_not_delete_original_failure(self):
        process = self.value["profiles"]["synthetic-candidate"][0]
        process["raw_records"].append({"type": "failure", "stage": "initialization", "status": "VisionFailed"})
        retain_raw(self.value, self.evidence_root)
        process["raw_records"].clear()
        verdict = self.verdict(retain=False)
        self.assertFalse(verdict["target_qualified"])
        self.assertTrue(any("projection mismatch: raw_records" in failure for failure in verdict["failures"]))
        self.assertTrue(any("VisionFailed" in failure for failure in verdict["failures"]))

    def test_report_operational_edit_cannot_erase_post_terminal_timeout(self):
        process = self.value["profiles"]["synthetic-candidate"][0]
        process["process"].update(exit_code=124, timed_out=True, cleanup_ok=False)
        retain_raw(self.value, self.evidence_root)
        process["process"].update(exit_code=0, timed_out=False, cleanup_ok=True)
        self.assertFalse(self.verdict(retain=False)["target_qualified"])

    def test_original_process_receipt_is_required(self):
        retain_raw(self.value, self.evidence_root)
        process = self.value["profiles"]["synthetic-candidate"][0]
        (self.evidence_root / process["receipt_path"]).unlink()
        self.assertFalse(self.verdict(retain=False)["target_qualified"])

    def test_process_receipt_digest_is_bound(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"]["synthetic-candidate"][0]["receipt_sha256"] = "0" * 64
        self.assertFalse(self.verdict(retain=False)["target_qualified"])

    def test_report_command_cannot_replace_original_invocation(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"]["synthetic-candidate"][0]["command"][-1] = "2"
        self.assertFalse(self.verdict(retain=False)["target_qualified"])

    def test_unreported_process_receipt_refuses_a_hidden_pre_jsonl_failure(self):
        retain_raw(self.value, self.evidence_root)
        (self.evidence_root / "failed-before-observation.process.json").write_bytes(
            report.document_bytes({"index": 0, "command": ["missing"], "process": {"launch_error": "missing"}}))
        self.assertFalse(self.verdict(retain=False)["target_qualified"])

    def test_missing_original_jsonl_refuses_despite_embedded_samples(self):
        retain_raw(self.value, self.evidence_root)
        path = self.evidence_root / self.value["profiles"]["synthetic-candidate"][0]["raw_path"]
        path.unlink()
        self.assert_refused("original raw JSONL unavailable", retain=False)

    def test_original_jsonl_digest_mismatch_refuses(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"]["synthetic-candidate"][0]["raw_sha256"] = "a" * 64
        self.assert_refused("original raw JSONL digest mismatch", retain=False)

    def test_missing_external_evidence_root_is_not_an_in_report_flag(self):
        retain_raw(self.value, self.evidence_root)
        self.value["raw_verified"] = True
        verdict = report.classify(self.value, repository_root=self.repository)
        self.assertFalse(verdict["target_qualified"])
        self.assertIn("original raw JSONL evidence root is required", verdict["failures"])

    def test_raw_path_cannot_escape_actual_evidence_root(self):
        retain_raw(self.value, self.evidence_root)
        self.value["profiles"]["synthetic-candidate"][0]["raw_path"] = "../outside.jsonl"
        self.assert_refused("unsafe relative evidence path", retain=False)

    def test_unreported_original_failed_attempt_refuses(self):
        retain_raw(self.value, self.evidence_root)
        failed = self.evidence_root / "failed-attempt-never-retried.jsonl"
        failed.write_text('{"type":"failure","status":"VisionFailed"}\n', encoding="utf-8")
        self.assert_refused("attempt set differs", retain=False)

    def test_extra_profile_cannot_hide_original_failed_attempt(self):
        process = copy.deepcopy(self.value["profiles"]["synthetic-candidate"][0])
        process["profile"] = "hidden-attempt"
        process["raw_records"].append({"type": "failure", "status": "VisionFailed"})
        self.value["profiles"]["hidden-attempt"] = [process]
        self.assert_refused("exact reference/candidate attempt set")

    def test_rehashing_all_embedded_budget_fields_cannot_forge_precommit(self):
        name = self.candidate_row()["workload"]
        self.value["budgets"]["workloads"][name]["max_ms"] *= 2
        evidence = self.value["policy_inputs"]["budgets"]
        forged = policy_evidence(policy_documents(self.value)["budgets"], evidence["revision"], evidence["path"])
        self.value["policy_inputs"]["budgets"] = forged
        self.value["budget_sha256"] = forged["sha256"]
        (self.repository / evidence["path"]).write_bytes(base64.b64decode(forged["base64"]))
        self.assert_refused("frozen budgets differs from historical Git tree/blob authority")

    def test_rehashing_all_embedded_host_fields_cannot_forge_precommit(self):
        self.value["host_policy"]["approved_cohorts"].append({"cohort": {}, "approval": "forged"})
        evidence = self.value["policy_inputs"]["host_policy"]
        forged = policy_evidence(self.value["host_policy"], evidence["revision"], evidence["path"])
        self.value["policy_inputs"]["host_policy"] = forged
        self.value["host_policy_sha256"] = forged["sha256"]
        (self.repository / evidence["path"]).write_bytes(base64.b64decode(forged["base64"]))
        self.assert_refused("frozen host_policy differs from historical Git tree/blob authority")

    def test_policy_without_repository_relative_path_refuses(self):
        del self.value["policy_inputs"]["budgets"]["path"]
        self.assert_refused("repository-relative policy path")

    def test_actual_frozen_tree_path_cannot_be_replaced_by_another_policy(self):
        self.value["policy_inputs"]["budgets"]["path"] = self.value["policy_inputs"]["host_policy"]["path"]
        self.assert_refused("historical Git tree/blob authority")

    def test_missing_original_policy_git_blob_refuses_embedded_copy(self):
        blob = self.value["policy_inputs"]["budgets"]["git_blob"]
        (self.repository / ".git" / "objects" / blob[:2] / blob[2:]).unlink()
        self.assert_refused("frozen budgets Git authority unavailable")

    def test_missing_git_repository_never_falls_back_to_embedded_bytes(self):
        retain_raw(self.value, self.evidence_root)
        verdict = report.classify(self.value, evidence_root=self.evidence_root, repository_root=self.root)
        self.assertFalse(verdict["target_qualified"])
        self.assertTrue(any("Git authority" in failure for failure in verdict["failures"]))

    def test_policy_committed_after_implementation_is_not_precommitted(self):
        old = copy.deepcopy(self.value["binding"])
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
        self.assertEqual(self.value["binding"]["source_revision"], old["source_revision"])
        self.assert_refused("merge-base --is-ancestor")

    def test_forged_source_tree_cannot_match_itself_in_build_and_binding(self):
        binding = self.value["binding"]
        binding["source_tree"] = binding["build_source_tree"] = "a" * 40
        self.assert_refused("source tree differs from Git commit authority")

    def test_old_build_cannot_claim_new_head_when_catalog_is_unchanged(self):
        original_catalog = self.value["binding"]["catalog_sha256"]
        old_build = self.value["binding"]["build_source_revision"]
        (self.repository / "implementation.rs").write_text("// synthetic implementation B\n", encoding="utf-8")
        git(self.repository, "add", ".")
        git(self.repository, "-c", "user.name=Synthetic Evidence", "-c", "user.email=synthetic@example.invalid",
            "-c", "commit.gpgsign=false", "commit", "-qm", "new implementation, unchanged catalog")
        revision, tree = git(self.repository, "rev-parse", "HEAD"), git(self.repository, "rev-parse", "HEAD^{tree}")
        self.value["binding"].update(source_revision=revision, source_tree=tree)
        for processes in self.value["profiles"].values():
            for process in processes:
                process["binding"] = copy.deepcopy(self.value["binding"])
        self.assertNotEqual(old_build, revision)
        self.assertEqual(self.value["binding"]["catalog_sha256"], original_catalog)
        self.assert_refused("compiled build provenance mismatch: build_source_revision")

    def test_compiler_build_provenance_cannot_be_replaced_by_current_rustc(self):
        self.value["binding"]["compiler"] = "new selected compiler"
        self.assert_refused("compiled build provenance mismatch: build_compiler")

    def test_dirty_or_unavailable_build_source_never_qualifies(self):
        self.value["binding"]["build_source_dirty"] = True
        self.assert_refused("compiled build source is dirty or unavailable")

    def test_unavailable_build_commit_never_qualifies(self):
        self.value["binding"]["build_source_revision"] = "unavailable"
        self.assert_refused("invalid binding build_source_revision")

    def test_unavailable_compiler_cannot_qualify_by_matching_sentinels(self):
        self.value["binding"]["compiler"] = self.value["binding"]["build_compiler"] = "unavailable"
        self.assert_refused("compiled build compiler is unavailable")

    def test_actual_uncommitted_source_remains_nonqualifying(self):
        (self.repository / "implementation.rs").write_text("// dirty implementation\n", encoding="utf-8")
        self.value["source_changes"] = git(self.repository, "status", "--porcelain", "--untracked-files=all")
        self.assertTrue(self.value["source_changes"])
        self.assert_refused("uncommitted implementation source")

    def test_cli_validate_reparses_original_jsonl_not_edited_report(self):
        row = self.candidate_row()
        self.value["budgets"]["workloads"][row["workload"]]["max_ms"] = 200
        freeze_policies(self.repository, (self.value, self.windows))
        row["metrics"]["elapsed_ms"] = 201
        retain_raw(self.value, self.evidence_root)
        row["metrics"]["elapsed_ms"] = 199
        path = self.evidence_root / "report.json"
        path.write_bytes(report.document_bytes(self.value))
        result = subprocess.run([sys.executable, str(Path(report.__file__).with_name("run.py")),
                                 "validate", str(path), "--repository", str(self.repository)],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8")
        self.assertEqual(result.returncode, 2, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertFalse(verdict["target_qualified"])
        self.assertTrue(any("projection mismatch" in failure for failure in verdict["failures"]))
        self.assertTrue(any("absolute budget max_ms" in failure for failure in verdict["failures"]))

    def test_cli_admission_requires_originals_on_both_targets(self):
        paths = []
        for value, root in ((self.value, self.evidence_root), (self.windows, self.windows_root)):
            retain_raw(value, root)
            path = root / "report.json"
            path.write_bytes(report.document_bytes(value))
            paths.append(path)
        command = [sys.executable, str(Path(report.__file__).with_name("run.py")), "admission",
                   *map(str, paths), "--repository", str(self.repository)]
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)["admitted"])
        (self.windows_root / self.windows["profiles"]["synthetic-candidate"][0]["raw_path"]).unlink()
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertFalse(json.loads(result.stdout)["admitted"])

    def test_geometry_iou_equality_has_no_stricter_hidden_gate(self):
        source = {key: "synthetic" for key in ("stamp", "transform", "effective_region", "backend", "output_space", "effective_zones")}
        expected = [{"text_nfc": "A", "source_relative_quad": [[.25, .25], [.5, .25], [.5, .5], [.25, .5]]}]
        observation = {"status": "Ok", "source_correlation": True, "source": source, "result_source": dict(source),
                       "regions": [{"text": "A", "confidence": 0, "quad": [[.125, .25], [.625, .25], [.625, .5], [.125, .5]]}], "groups": None}
        self.assertEqual(report.quality(observation, expected, "singular"), [])


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
