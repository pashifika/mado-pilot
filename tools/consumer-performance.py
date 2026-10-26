#!/usr/bin/env python3
"""Run one source-bound actual-consumer case, never a chained campaign.

CLI: prepare --source-plan FILE --case ID --campaign DIR --output FILE
     {smoke,calibrate,control,compare} --plan FILE --output NEW_DIR
     freeze --calibration DIR --control DIR --review FILE --output NEW_FILE
     report DIR
     inspect --plan FILE --raw RETAINED_RECORD [--sink-root DIR]
     discover --plan FILE --role {baseline,candidate} --output NEW_DIR
     close-runtime --plan FILE --discovery FILE [--discovery FILE] --output NEW_FILE

prepare imports only commands/identities from an explicit retained development
plan; it never imports its samples as calibration. Edit the NEW plan prospectively
before numerical collection. Plans use the twelve CASES below, explicit argv,
cwd/environment, hash-pinned inputs and binaries, and one persistent campaign
root per target/host. No implicit environment, build, download, retry or resume.
Numerical stages freeze both role identities in advance. A control plan names
calibration as {"directory": "...", "binding": {"path": ".../summary.json",
"sha256": "..."}}; comparison names profile as {"path": "...", "sha256": "..."}.
profile-proposal prints the exact proposal digest for the independent freeze review.

Numerical admission is a hash-bound review (kind consumer-method-review-v4),
with context_sha256=context_digest(plan), accepted=true, reviewer/rationale,
metric contracts, and evidence for source, runtime_images, owned_processes,
observer_cost, clocks_cpu, resources, retention and exclusive_host. Each gate
names its remaining issues and hash-bound evidence files; unknown is not passed.
v1/v2/v3 reviews are refused, never reinterpreted.
Each selected resident bytes metric declares exact hard_ceiling_bytes and
comparison_allowance_floor_bytes; bound resources evidence repeats them under
resident_policies with a justification. Every invocation obeys that fixed point
ceiling; comparisons use max(calibration spread, floor). No peak/growth/residual.
Trial operation_us declares policy fresh-trial-central-four-v1, population
fresh-process-first-operation, exact hard_ceiling_us and max_regression_us;
bound resources evidence repeats them under trial_responsiveness with a
justification. Every trial operation obeys that ceiling. Only repeatability
uses the central four of six values after removing one minimum and maximum.
All-six paired non-regression, order, drift and median checks remain intact.
Excess repeatability is inconclusive; candidate excess is an observed
non-regression violation. Trials carry no benefit obligation; group/matched do.
prepare --loader [--node ABSOLUTE_BINARY] enables actual Apple dyld logging.
discover invokes one selected real command; close-runtime unions only its explicit
hash-bound raw/sink discoveries and rechecks images/cache. Neither grants numerical
approval. Bind its immutable output as plan.runtime_closure before method review.
The review also binds source_closed_cpu and accounting_allowance evidence references.
CPU retains raw partial/null fields; source-closed eligibility and the prospective
margin are separate, never physical-resolution claims. Native reachable process
paths need case/source/artifact evidence, not library-wide guesses or dyld EOF.
Development smoke remains runnable without qualification gates and retains all
missing reasons. Its numerical misses are descriptive, not execution failures.
Windows runtime/CPU method admission remains unavailable.

All JSON receipts/profiles use exclusive creation. A two-attempt checkpoint can
pause the same live generator; the original monotonic deadline keeps running.
Closing or interrupting it finalizes an incomplete phase, not a resumable cursor.
Campaign registrations outlive phase owners; an unfinished registration, identity
or cleanup failure fences later launches. Failed controls prevent qualification;
sealed, purely numerical failures permit separately registered diagnostics.
This collector never clears fences or retries a qualification cohort.
Current methods: docs/performance/methods/actual-consumer.md.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import importlib.util
import math
import os
from pathlib import Path
import shutil
import stat
import sys
import time
from fractions import Fraction

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "consumer_component_utilities", ROOT / "tools/template-matching-efficiency.py")
component = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(component)
from process_runner import run_process  # noqa: E402
import consumer_runtime as runtime  # noqa: E402

require = component.require
integer = component.integer
nonempty = component.nonempty
strict_json = component.strict_json
document = component.document
sha256 = component.sha256
host_identity = component.host_identity
MIB = 1024 * 1024
CONFIGURATION = {"opt_level": "0", "debuginfo": 2, "debug_assertions": True,
                 "benchmark_accounting": False, "observations": True}
CASES = tuple(f"{kind}-{area}" for kind in ("replay", "group", "matched", "deadline")
              for area in ("full", "roi")) + tuple(
    f"trial-{outcome}-{area}" for outcome in ("hit", "miss") for area in ("full", "roi"))
STAGES = {"smoke": 1, "calibration": 6, "control": 12, "comparison": 12}
TOTAL_SECONDS = {"smoke": 900, "calibration": 900, "control": 1680, "comparison": 1680}
LIMITS = {"primary_seconds": 90, "cleanup_seconds": 5, "retention_seconds": 10,
          "output_limit_bytes": 4 * MIB, "child_sink_bytes": 8 * MIB,
          "admission_seconds": 60, "final_retention_seconds": 60}
GATES = ("source", "runtime_images", "owned_processes", "observer_cost", "clocks_cpu",
         "resources", "retention", "exclusive_host")
METHOD_REVIEW = "consumer-method-review-v4"
TRIAL_POLICY = "fresh-trial-central-four-v1"
TRIAL_POPULATION = "fresh-process-first-operation"
TRIAL_FIELDS = ("policy", "population", "hard_ceiling_us", "max_regression_us")
TRIAL_MARKS = ("entered", "validated", "resources_verified", "init_started", "initialized",
               "resources_reverified", "open_started", "opened", "acquired", "prepared",
               "search_started", "search_finished", "projected", "primary_returned", "settled",
               "close_started", "closed", "owner_release_started", "owner_released")
CALL_NAMES = ("asset", "observe", "recognize", "scan_ocr_zones", "query", "query_wait",
              "submit", "settle", "postcondition", "release", "wait", "log", "fixture",
              "target_start", "target_status", "unknown")


class ContractError(ValueError):
    def __init__(self, category, reason):
        super().__init__(reason)
        self.category = category


def check(condition, category, reason):
    if not condition:
        raise ContractError(category, reason)


def issue(category, reason):
    return {"category": category, "reason": reason}


def exact(value):
    require(type(value) in (int, str, float, Fraction), "numeric observation unavailable")
    if isinstance(value, float):
        require(math.isfinite(value), "nonfinite observation")
    result = Fraction(str(value))
    require(result >= 0, "negative observation")
    return result


def median(values):
    values = sorted(exact(value) for value in values)
    require(bool(values), "empty median")
    middle = len(values) // 2
    return values[middle] if len(values) % 2 else (values[middle - 1] + values[middle]) / 2


def hashed(value):
    return hashlib.sha256(document(value)).hexdigest()


def load(path, maximum=4 * MIB):
    path = Path(path)
    require(path.stat().st_size <= maximum, "JSON exceeds retained bound: " + str(path))
    return strict_json(path.read_text(encoding="utf-8"))


def save(path, value, maximum=4 * MIB):
    payload = document(value)
    require(len(payload) <= maximum, "receipt exceeds reserved bytes: " + str(path))
    with Path(path).open("xb") as stream:
        stream.write(payload)
        stream.flush()
        os.fsync(stream.fileno())
    return {"path": str(Path(path).resolve()), "sha256": hashlib.sha256(payload).hexdigest(),
            "bytes": len(payload)}


def file_digest(path, deadline=None):
    path = Path(path)
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode) and before.st_size <= 512 * MIB,
            "bounded regular input required: " + str(path))
    result = hashlib.sha256()
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        while True:
            if deadline is not None:
                if time.monotonic() > deadline:
                    raise TimeoutError("input verification deadline")
            block = stream.read(MIB)
            if not block:
                break
            result.update(block)
        after = os.fstat(stream.fileno())
    final = path.stat()
    identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns)
    require(identity(before) == identity(opened) == identity(after) == identity(final),
            "input changed while hashing: " + str(path))
    return result.hexdigest()


def reference(path):
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": file_digest(path)}


def read_reference(binding, maximum=4 * MIB):
    require(isinstance(binding, dict), "missing evidence reference")
    component.digest(binding.get("sha256"), "evidence sha256")
    path = Path(binding["path"])
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode) and before.st_size <= 512 * MIB,
            "bounded regular input required: " + str(path))
    require(before.st_size <= maximum, "JSON exceeds retained bound: " + str(path))
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        raw = stream.read(maximum + 1)
        after = os.fstat(stream.fileno())
    final = path.stat()
    identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns)
    require(identity(before) == identity(opened) == identity(after) == identity(final),
            "input changed while reading: " + str(path))
    require(len(raw) <= maximum, "JSON exceeds retained bound: " + str(path))
    require(hashlib.sha256(raw).hexdigest() == binding["sha256"], "evidence identity changed")
    return strict_json(raw.decode("utf-8"))


def collector_identity():
    paths = [Path(__file__).resolve(), ROOT / "tools/template-matching-efficiency.py"]
    paths += [ROOT / "tools/native-release-profile" / name for name in
              ("process_runner.py", "_process_group.py", "_windows_process.py", "macos_image_observer.py", "consumer_runtime.py")]
    return {str(path.relative_to(ROOT)): sha256(path) for path in paths}


def schedule(stage):
    require(stage in STAGES, "unknown phase")
    if stage in ("smoke", "calibration"):
        return [{"id": f"{index:02d}", "role": "baseline", "pair": index // 2,
                 "position": index % 2, "process": index} for index in range(STAGES[stage])]
    roles = ("baseline", "control" if stage == "control" else "candidate")
    rows = []
    for pair in range(6):
        for position, role in enumerate(roles if pair % 2 == 0 else roles[::-1]):
            rows.append({"id": f"{len(rows):02d}", "role": role, "pair": pair,
                         "position": position, "process": pair})
    return rows


def context_digest(plan):
    # Paths are intentionally bound: another source/build/output root is not the same cohort.
    keys = ("case", "target", "host", "configuration", "root", "env", "inputs", "roles",
            "collector", "bounds", "fixture", "loader", "campaign")
    return hashed({**{key: plan[key] for key in keys},
                   "runtime_closure": plan.get("runtime_closure")})


def validate_plan(plan):
    require(plan.get("format") == "actual-consumer-plan-v1", "unsupported consumer plan")
    require(plan.get("execution_authorized") is True, "explicit execution authorization required")
    require(plan.get("case") in CASES, "one of twelve fixed cases required")
    require(plan.get("target") in component.TARGETS, "unsupported native target")
    require(plan.get("configuration") == CONFIGURATION, "dev-normal observation-on configuration required")
    require(isinstance(plan.get("env"), dict) and all(isinstance(k, str) and isinstance(v, str)
            for k, v in plan["env"].items()), "explicit string environment required")
    require(plan["env"].get("MADO_P050_OBSERVATIONS") == "1", "observation-on required")
    require("MADO_P050_STDERR_SINK_DIR" not in plan["env"], "collector owns child sink directory")
    require(plan["env"].get("DYLD_PRINT_LIBRARIES") in (None, "1") and
            (plan["env"].get("DYLD_PRINT_LIBRARIES") == "1") == plan["loader"], "loader mode mismatch")
    require(type(plan["loader"]) is bool, "explicit loader mode required")
    require(Path(plan["campaign"]).is_absolute() and Path(plan["root"]).is_absolute(),
            "explicit absolute campaign/cwd required")
    require(isinstance(plan["inputs"], dict) and 0 < len(plan["inputs"]) <= 20000,
            "bounded source/input manifest required")
    for path, binding in plan["inputs"].items():
        require(Path(path).is_absolute(), "absolute input path required")
        component.digest(binding.get("sha256"), "input sha256")
    require(set(plan["bounds"]) == set(LIMITS), "complete execution bounds required")
    for name, maximum in LIMITS.items():
        integer(plan["bounds"][name], name, 1)
        require(plan["bounds"][name] <= maximum, "execution bound exceeds accepted method: " + name)
    require(plan["bounds"]["cleanup_seconds"] == 5 and
            plan["bounds"]["retention_seconds"] == 10 and
            plan["bounds"]["final_retention_seconds"] == 60 and
            plan["bounds"]["admission_seconds"] == 60, "cleanup/retention reservation must not shrink")
    require("baseline" in plan["roles"] and set(plan["roles"]) <= {"baseline", "candidate"},
            "baseline and optional frozen candidate roles required")
    for role in plan["roles"].values():
        argv = role["argv"]
        require(isinstance(argv, list) and argv and all(isinstance(arg, str) and arg for arg in argv),
                "explicit argv required")
        for key in ("engine", "root_binary"):
            binary = role[key]
            require(plan["inputs"].get(binary["path"], {}).get("sha256") == binary["sha256"],
                    "binary absent from input manifest")
            profile = binary.get("cargo_artifact", {}).get("profile", {})
            require(profile.get("opt_level") == "0" and profile.get("debuginfo") == 2
                    and profile.get("debug_assertions") is True, "actual consumer dev build identity required")
        require(argv[0] == role["root_binary"]["path"], "argv root binary mismatch")
        if plan["case"].startswith("trial-"):
            require(len(argv) == 9 and argv[1::2] == ["--engine", "--fixture", "--case", "--environment"],
                    "saved-frame trial command required")
            require(argv[2] == role["engine"]["path"] and argv[6] == plan["case"][6:],
                    "trial command case/engine mismatch")
            require(argv[4] in plan["inputs"] and argv[8] in plan["inputs"], "trial assets not bound")
        else:
            require(len(argv) == 4 and argv[1] == "run" and argv[0] == role["engine"]["path"],
                    "actual supervised engine run required")
            require(argv[2] in plan["inputs"], "replay plan not bound")
    component.digest(plan["fixture"]["corpus_sha256"], "corpus sha256")
    require(isinstance(plan.get("admission"), (dict, type(None))), "invalid admission reference")
    require(isinstance(plan.get("runtime_closure"), (dict, type(None))), "invalid runtime closure reference")
    if plan["loader"]:
        runtime.images._check_loader_env({key: value for key, value in plan["env"].items() if key.startswith("DYLD_")})


def verify_inputs(plan, deadline):
    failures = []
    for path, binding in plan["inputs"].items():
        try:
            require(file_digest(path, deadline) == binding["sha256"], "digest mismatch")
        except TimeoutError as error:
            failures.append(issue("unavailable", f"post/pre input verification incomplete: {path}: {error}"))
            break
        except (ValueError, OSError) as error:
            failures.append(issue("identity", f"{path}: {error}"))
            if time.monotonic() > deadline:
                break
    if collector_identity() != plan["collector"]:
        failures.append(issue("identity", "collector/runner source changed"))
    if host_identity() != plan["host"] or host_identity()["target"] != plan["target"]:
        failures.append(issue("identity", "target/host changed"))
    return failures


def required_metrics(case):
    metrics = {"lifetime_cpu_seconds": "s"}
    kind = case.split("-")[0]
    if kind == "trial":
        metrics["operation_us"] = "us"
    elif kind == "replay":
        metrics["invocation_elapsed_seconds"] = "s"
        metrics.update({key: "us" for key in
                        ("startup_us", "elapsed_us", "preflight_us", "workflow_us", "cleanup_us")})
    elif kind == "group":
        metrics["group_span_ns"] = "ns"
        for index in range(8):
            metrics[f"recognize_{index}_ns"] = "ns"
            metrics[f"release_{index}_ns"] = "ns"
    elif kind == "matched":
        metrics["query_wait_ns"] = "ns"
    return metrics


def selectable_metrics(case):
    metrics = {**required_metrics(case), "invocation_elapsed_seconds": "s"}
    if not case.startswith("trial-"):
        metrics.update({key: "us" for key in
                        ("startup_us", "elapsed_us", "preflight_us", "workflow_us", "cleanup_us")})
        metrics.update(child_sampled_rss_bytes="bytes", supervisor_sampled_rss_bytes="bytes")
        if case.startswith("deadline-"):
            metrics["query_wait_ns"] = "ns"
    else:
        metrics["recognition_child_rss_bytes"] = "bytes"
    return metrics


def benefit_metric(case):
    # Fresh trials prove responsiveness/non-regression only, never a positive benefit.
    return {"group": "group_span_ns", "matched": "query_wait_ns"}.get(case.split("-")[0])


def trial_metric(case, name):
    return case.startswith("trial-") and name == "operation_us"


def semantic_output(plan, role, record):
    require(record["exit_code"] == int(plan["case"].startswith("deadline-"))
            and record["cleanup_ok"] is True and record["timed_out"] is False
            and record["output_limited"] is False and record["launch_error"] is None,
            "discovery outer invocation did not complete its selected contract")
    output = strict_json(record["stdout"])
    selected = plan["roles"][role]
    require(not terminal_issues(plan, selected, output), "discovery terminal identity/cleanup failed")
    checked = (trial_check(plan, selected, output, {}) if plan["case"].startswith("trial-")
               else replay_check(plan, selected, output))
    if checked["driver_declared_outer"] is not None:
        embedded = checked["driver_declared_outer"]
        require(embedded["outer_primary_ms"] <= plan["bounds"]["primary_seconds"] * 1000
                and embedded["outer_cleanup_ms"] == plan["bounds"]["cleanup_seconds"] * 1000
                and embedded["outer_output_bytes"] <= plan["bounds"]["output_limit_bytes"],
                "invoking envelope is smaller than driver-declared outer contract")
    return output


def runtime_bundle(plan):
    require(plan.get("runtime_closure") is not None, "runtime_images: actual reviewed runtime closure missing")
    bundle = read_reference(plan["runtime_closure"])
    return runtime.validate_bundle(plan, bundle, read_reference, semantic_output)


def accounting_method(plan, review, bundle):
    require(review is not None and review.get("accepted") is True
            and review.get("context_sha256") == context_digest(plan), "accepted source-closed method review missing")
    certificate = read_reference(review.get("source_closed_cpu"))
    runtime.validate_certificate(plan, certificate, bundle, context_digest(plan), read_reference)
    allowance = read_reference(review.get("accounting_allowance"))
    value = runtime.validate_allowance(plan, allowance, review["source_closed_cpu"], context_digest(plan), read_reference)
    return certificate, value


def capture_runtime(plan, bundle, role, deadline):
    try:
        if time.monotonic() > deadline:
            raise TimeoutError("runtime identity verification deadline")
        return {"value": runtime.snapshot(plan, bundle, role, deadline), "issues": []}
    except TimeoutError as error:
        return {"value": None, "issues": [issue("unavailable", "runtime post/pre verification incomplete: " + str(error))]}
    except (KeyError, TypeError, ValueError, OSError, runtime.images._Refusal) as error:
        return {"value": None, "issues": [issue("identity", "runtime identity verification: " + str(error))]}


def resident_bounds(value):
    """Declared fixed point ceiling and comparison floor; exact JSON integers, never sample-derived."""
    ceiling = integer(value.get("hard_ceiling_bytes"), "hard_ceiling_bytes", 1)
    floor = integer(value.get("comparison_allowance_floor_bytes"), "comparison_allowance_floor_bytes", 1)
    require(floor <= ceiling, "comparison allowance floor exceeds hard ceiling")
    return ceiling, floor


def resident_policies(value, resident):
    """One resources-gate policy map keyed exactly by the selected resident metrics."""
    require(isinstance(value, dict) and resident and set(value) == resident,
            "resident policies must name exactly the selected resident metrics")
    result = {}
    for metric, policy in value.items():
        require(isinstance(policy, dict), metric + ": resident policy object required")
        nonempty(policy.get("justification"), metric + " resident policy justification")
        result[metric] = resident_bounds(policy)
    return result


def trial_budget(value):
    """Declared fresh-trial policy, ceiling and absolute tolerance; exact JSON integers, never sample-derived."""
    require(isinstance(value, dict), "trial responsiveness object required")
    require(value.get("policy") == TRIAL_POLICY and value.get("population") == TRIAL_POPULATION,
            "fresh-trial responsiveness policy/population required")
    ceiling = integer(value.get("hard_ceiling_us"), "hard_ceiling_us", 1)
    tolerance = integer(value.get("max_regression_us"), "max_regression_us", 1)
    require(tolerance <= ceiling, "max_regression_us exceeds hard_ceiling_us")
    return {"policy": TRIAL_POLICY, "population": TRIAL_POPULATION,
            "hard_ceiling_us": ceiling, "max_regression_us": tolerance}


def trial_contract(contract):
    """Budget and clock of one trial operation contract; 2*resolution must fit the tolerance."""
    budget = trial_budget(contract)
    quantum, resolution = exact(contract.get("quantum")), exact(contract.get("resolution"))
    require(quantum > 0 and resolution > 0, "positive quantum and measured effective resolution required")
    require(2 * resolution <= budget["max_regression_us"], "2*resolution exceeds max_regression_us")
    return budget, (quantum, resolution)


def admission(plan):
    failures = []
    review = None
    if plan.get("admission") is not None:
        try:
            review = read_reference(plan["admission"])
            require(isinstance(review, dict) and isinstance(review.get("gates"), dict)
                    and isinstance(review.get("metrics"), dict), "method review gates/metrics objects required")
            require(review.get("kind") == METHOD_REVIEW and review.get("accepted") is True,
                    "accepted consumer method review required")
            require(review.get("context_sha256") == context_digest(plan), "method review context mismatch")
            nonempty(review.get("reviewer"), "reviewer")
            nonempty(review.get("rationale"), "rationale")
        except (KeyError, TypeError, ValueError, OSError) as error:
            failures.append(issue("admission", str(error)))
            review = None
    contracts = (review or {}).get("metrics", {})
    selected = {**required_metrics(plan["case"]), **{
        name: selectable_metrics(plan["case"]).get(name) for name in contracts}}
    resident = {name for name, unit in selected.items() if unit == "bytes"}
    policies, budgets = [], []
    for name in GATES:
        gate = (review or {}).get("gates", {}).get(name, {})
        if (not isinstance(gate, dict) or gate.get("status") != "accepted" or gate.get("issues") != []
                or not isinstance(gate.get("evidence"), list) or not gate["evidence"]):
            failures.append(issue("unavailable", name + ": source-bound accepted evidence missing"))
            continue
        for binding in gate["evidence"]:
            try:
                evidence = read_reference(binding)
                require(isinstance(evidence, dict) and isinstance(evidence.get("source_files"), dict),
                        "gate evidence/source-file objects required")
                require(evidence.get("context_sha256") == context_digest(plan), "evidence context mismatch")
                require(evidence.get("gate") == name and evidence.get("complete") is True,
                        "incomplete gate evidence")
                require(evidence.get("source_files") and all(plan["inputs"].get(path, {}).get("sha256") == digest
                        for path, digest in evidence["source_files"].items()), "gate source binding missing")
                if name == "runtime_images":
                    require(evidence.get("closure") == plan.get("runtime_closure") and evidence.get("closure") is not None,
                            "exact actual runtime closure reference required")
                if name == "owned_processes":
                    require(evidence.get("source_certificate") == review.get("source_closed_cpu")
                            and evidence.get("source_certificate") is not None,
                            "actual source-closed owned roles required; Job totals are not an identified census")
                if name == "clocks_cpu":
                    require(evidence.get("accounting_allowance") == review.get("accounting_allowance")
                            and evidence.get("accounting_allowance") is not None,
                            "prospective source/host/API-bound accounting allowance missing")
                if name == "resources" and "resident_policies" in evidence:
                    policies.append(resident_policies(evidence["resident_policies"], resident))
                if name == "resources" and "trial_responsiveness" in evidence:
                    require(plan["case"].startswith("trial-"), "fresh-trial responsiveness policy is trial-only")
                    declared = trial_budget(evidence["trial_responsiveness"])
                    nonempty(evidence["trial_responsiveness"].get("justification"), "trial responsiveness justification")
                    budgets.append(declared)
            except (KeyError, TypeError, ValueError, OSError) as error:
                failures.append(issue("unavailable", name + ": " + str(error)))
    for metric, unit in selected.items():
        contract = contracts.get(metric, {})
        try:
            require(isinstance(contract, dict), "metric contract object required")
            require(unit is not None, "unsupported metric")
            require(contract.get("unit") == unit and contract.get("missing_reason") is None,
                    "metric unit/availability mismatch")
            for key in ("api", "owner", "interval"):
                nonempty(contract.get(key), key)
            if trial_metric(plan["case"], metric):
                budget = trial_contract(contract)[0]
                require(budget["hard_ceiling_us"] < exact(plan["bounds"]["primary_seconds"]) * 1_000_000,
                        "hard_ceiling_us must be below the outer process timeout")
                require(budgets and all(declared == budget for declared in budgets),
                        "bound resources evidence must declare the same fresh-trial responsiveness budget")
            else:
                require(not set(TRIAL_FIELDS) & set(contract),
                        "fresh-trial responsiveness policy applies only to trial operation_us")
            if unit == "bytes":
                require(contract.get("domain") == "resident", "resource point is not allocation/owner residual")
                require(not {"quantum", "resolution", "resolution_method"} & set(contract),
                        "resident byte point has no timer quantum/resolution")
                bounds = resident_bounds(contract)
                require(policies and all(policy[metric] == bounds for policy in policies),
                        "bound resources evidence must declare the same resident ceiling/floor")
            elif metric == "lifetime_cpu_seconds":
                require(contract.get("api") == "posix-wait4"
                        and contract.get("accounting_allowance") == (review or {}).get("accounting_allowance")
                        and contract.get("accounting_allowance") is not None
                        and exact(contract.get("quantum")) == exact("1/1000000")
                        and "resolution" not in contract and "resolution_method" not in contract,
                        "CPU grading margin must remain distinct from raw resolution")
            else:
                nonempty(contract.get("resolution_method"), "resolution_method")
                require(exact(contract.get("quantum")) > 0 and exact(contract.get("resolution")) > 0,
                        "positive quantum and measured effective resolution required")
        except (ValueError, TypeError) as error:
            failures.append(issue("unavailable", metric + ": " + str(error)))
    try:
        bundle = runtime_bundle(plan)
    except (KeyError, IndexError, TypeError, ValueError, OSError, runtime.images._Refusal) as error:
        failures.append(issue("unavailable", "runtime_images: " + str(error)))
    else:
        try:
            accounting_method(plan, review, bundle)
        except (KeyError, IndexError, TypeError, ValueError, OSError, runtime.images._Refusal) as error:
            failures.append(issue("unavailable", "owned_processes/clocks_cpu: " + str(error)))
    return review, failures


def thread_check(value):
    require(value["supported"] is True and integer(value["configured_threads"], "thread setting", 1),
            "native thread getter unavailable")
    require(value["active_concurrency"] is None and value["during_workload_invariance_observed"] is False,
            "native setting is only a point observation")
    require(integer(value["finished_us"], "getter end") >= integer(value["started_us"], "getter start"),
            "getter order")


def trial_check(plan, role, output, metrics):
    case = plan["case"][6:]
    require(output["version"] == 1 and output["scope"] == "supervised-saved-rgba-template-trial",
            "actual saved-frame schema required")
    check(output["case"] == case and output["corpus_sha256"] == plan["fixture"]["corpus_sha256"],
          "identity", "trial case/corpus identity")
    require(not any(output[key] for key in ("ui_coverage", "native_capture", "native_input")),
            "saved-frame scope changed")
    fixture = output["fixture_case"]
    expected_region = [64, 48, 320, 180] if case.endswith("roi") else None
    hit = case.startswith("hit")
    require(fixture["id"] == case and fixture["max_results"] == 8 and fixture["threshold"] == 0.9
            and fixture["search_region"] == expected_region, "authored trial options")
    require(fixture["frame"]["width"] == 1920 and fixture["frame"]["height"] == 1080
            and fixture["frame"]["bytes"] == 1920 * 1080 * 4
            and fixture["template"]["width"] == 12 and fixture["template"]["height"] == 10,
            "RGBA/template fixture dimensions")
    for key, arg in (("fixture", 4), ("environment", 8)):
        binding = output["inputs"][key]
        check(binding["path"] == role["argv"][arg] and
              binding["sha256"] == plan["inputs"][role["argv"][arg]]["sha256"], "identity", "trial input binding")
    for key in ("frame", "template"):
        check(output["inputs"][key]["sha256"] == fixture[key]["sha256"], "identity", "trial asset identity")
    operation = output["operation"]
    require(operation["accepted"] is True and operation["cancellation_requested"] is False
            and operation["stop_error"] is None and operation["ordinary_logs_omitted"] == 0,
            "trial accepted terminal without cancellation/log loss")
    check(operation["shutdown"] == {"completed": True, "error": None}
          and operation["terminal_observed"] is True and operation["requires_parent_owned_tree_cleanup"] is False,
          "cleanup", "trial public shutdown/settlement")
    view = operation["controller"]
    require(view["state"] == "terminal" and view["error"] is None and view["dropped_logs"] == 0,
            "controller settlement")
    check(view["run"] == operation["run"] and view["operation"] == "recognition_trial", "identity", "controller run identity")
    require(sum(event["event"] == "ChildStarted" for event in view["progress"]) == 2,
            "capability and recognition starts required")
    require(len(view["attempts"]) == 1, "exactly one capability child")
    result, probe = view["result"], view["attempts"][0]
    check(result["identity"] == operation["identity"], "identity", "result request identity")
    component.digest(result["environment_identity"], "environment identity")
    check(result["child_pid"] != probe["child_pid"], "identity", "separate fresh capability/recognition children")
    for child, expected in ((probe, "recognition_capabilities"), (result, "recognition_trial")):
        require(child["version"] == 1 and child["operation"] == expected and child["primary"] is None,
                "child terminal contract")
        integer(child["child_pid"], "child PID", 1)
        check(child["child_reaped"] is True and child["forced"] is False and child["exit_code"] == 0
              and child["cleanup"]["clean"] is True and child["cleanup"]["session_closed"] is True
              and child["cleanup"]["status"] == "CleanupFinished", "cleanup", "trial child ownership")
        check(child["build"]["executable_sha256"] == role["engine"]["sha256"]
              and child["build"]["target"] == plan["target"], "identity", "actual child binary/target")
        parent = child["supervision_observation"]
        require(parent["format"] == "trial-supervision-marks-v1" and parent["epoch"] == "parent-control.started"
                and parent["unit"] == "ns", "separate parent epoch")
        for names in (("spawn_started_ns", "spawn_returned_ns", "child_started_received_ns", "terminal_received_ns", "streams_joined_ns"),
                      ("spawn_started_ns", "spawn_returned_ns", "root_wait_returned_ns", "streams_joined_ns")):
            values = [integer(parent[name], name) for name in names]
            require(values == sorted(values), "parent lifecycle causal chain")
    check(result["cleanup"]["close_error"] is None, "cleanup", "trial close error")
    # Retain the identity/terminal-bound duration even if a later result or RSS oracle fails.
    metrics["operation_us"] = integer(operation["total_duration_us"], "trial operation")
    projection = result["result"]
    require(projection["id"] == "p050-template-0" and projection["kind"] == "template"
            and projection["threshold"] == 0.9 and projection["outcome"] == ("matched" if hit else "no_match"),
            "template result projection")
    require(len(projection["matches"]) == int(hit), "complete exact match count")
    if hit:
        found = projection["matches"][0]
        require(found["bounds"] == {"x": 80, "y": 64, "width": 12, "height": 10}
                and 0.99999 <= found["score"] <= 1, "planted template oracle")
    measure = result["measurement"]
    require(measure["format"] == "trial-stage-marks-v1" and measure["epoch"] == "control.started"
            and measure["unit"] == "ns" and measure["unavailable"] == [], "trial stage coverage")
    require(measure["names"] == list(TRIAL_MARKS), "exact stage names/order")
    marks = [integer(value, "stage mark") for value in measure["marks"]]
    require(len(marks) == 19 and marks == sorted(marks), "ordered complete trial stages")
    thread_check(measure["native_threads_after"])
    rss = measure.get("process_rss_after")
    if rss is not None:
        check(rss["pid"] == result["child_pid"], "identity", "RSS child identity")
        require(rss["method"] == "native-self-rss-v1" and rss["point"] == "after_trial_owner_release"
                and rss["scope"] == "recognition-child-process-point" and rss["epoch"] == "control.started"
                and rss["clock_unit"] == "ns", "RSS observation boundary")
        if rss["available"] is True:
            require(rss["reason"] is None and rss["native_os_error"] is None
                    and marks[-1] <= integer(rss["started_ns"], "RSS start") <= integer(rss["finished_ns"], "RSS end"),
                    "RSS bracket after owner release")
            metrics["recognition_child_rss_bytes"] = integer(rss["rss_bytes"], "RSS", 1)
    return {"metrics": metrics, "application_status": output["status"], "primary": None,
            "selected_pids": [probe["child_pid"], result["child_pid"]], "driver_declared_outer": output["bounds"]}


def operation_series(obs):
    series = obs["operation_samples"]
    require(series["format"] == "host-call-records-v1" and series["epoch"] == "control.started"
            and series["unit"] == "ns" and series["dropped"] == 0 and series["capacity"] == 64,
            "complete observation-on call records")
    require(series["names"] == list(CALL_NAMES) and len(series["records"]) <= 64, "call record dictionary/bound")
    grouped = {name: [] for name in CALL_NAMES}
    calls = []
    previous = 0
    for method, start, end, ok in series["records"]:
        integer(method, "method index")
        require(method < len(CALL_NAMES) and type(ok) is bool, "call index/outcome")
        require(previous <= integer(start, "call start") <= integer(end, "call end"), "call epoch order")
        previous = end
        name = CALL_NAMES[method]
        grouped[name].append((end - start, ok))
        calls.append((name, start, end, ok))
    for name, values in grouped.items():
        aggregate = obs["operation_metrics"][name]
        require(aggregate == {"count": len(values), "failures": sum(not ok for _, ok in values),
                              "total_us": sum(ns // 1000 for ns, _ in values),
                              "max_us": max((ns // 1000 for ns, _ in values), default=0)},
                "raw calls disagree with original aggregate: " + name)
    return calls


def replay_check(plan, role, output):
    require(output["version"] == 1 and len(output["runs"]) == 1, "one actual supervised run")
    report = output["runs"][0]
    kind, variant = plan["case"].split("-")
    deadline = kind == "deadline"
    require(report["status"] == ("FAIL" if deadline else "PASS") and report["exit_code"] == 0,
            "original application status/engine-child exit")
    primary = report["primary"]
    if deadline:
        require(primary["category"] == "Timeout" and primary["context"]["cause"]["status"] == "deadline_exceeded",
                "original application deadline fault")
    else:
        require(primary is None, "unexpected application fault")
    obs = report["observations"]
    cleanup = report["cleanup"]
    check(cleanup["clean"] is True and cleanup["status"] == "CleanupFinished"
          and cleanup["engine"]["session_closed"] is True and cleanup["engine"]["clean"] is True
          and cleanup["engine"]["close_error"] is None and cleanup["engine"]["physical_cleanup_confirmed"] is True
          and obs["child_reaped"] is True and report["forced"] is False, "cleanup", "replay close/reap")
    check(all(value == 0 for value in cleanup["remaining"].values())
          and all(obs[key] == 0 for key in ("attempt_owners", "live_handles", "in_flight_native", "queue_depth", "dropped_logs")),
          "cleanup", "released replay owners")
    check(report["build"]["executable_sha256"] == role["engine"]["sha256"]
          and report["build"]["target"] == plan["target"], "identity", "actual replay binary/target")
    require(obs["startup_settled"] is True and obs["terminal_accounted"] is True, "replay terminal accounting")
    logs = [strict_json(line) for line in obs["logs"]]
    check(all(row["variant"] == variant and row["corpus_sha256"] == plan["fixture"]["corpus_sha256"] for row in logs),
          "identity", "replay variant/corpus")
    stages = [row["stage"] for row in logs]
    metrics = {key: integer(report["metrics"][key], key) for key in
               ("startup_us", "elapsed_us", "preflight_us", "workflow_us", "cleanup_us")}
    calls = operation_series(obs)
    thread_check(obs["native_threads_after"])
    counts = obs["operation_metrics"]
    if kind in ("matched", "deadline"):
        expected = {"observe": 3, "recognize": 1, "query": 1, "query_wait": 1}
        require(stages[:3] == ["readiness", "confirmed-no-match", "query-wait-enter"], "query entry markers")
        require(logs[1]["matched"] is False and logs[2]["caller_observation_released"] is True
                and logs[2]["timeout_ms"] == (1 if deadline else 5000), "query ownership/deadline")
        require(logs[0]["frame"] == 1 and logs[1]["frame"] == 2 and logs[2]["source_frame"] == 2
                and all(row["query_kind"] == kind for row in logs), "query source correlation")
        if deadline:
            require(len(logs) == 3 and obs["recognitions"] == 1, "deadline must not acquire a result")
        else:
            require(stages[3:] == ["query-matched", "complete"] and counts["release"]["count"] == 6,
                    "matched query settlement")
            require(logs[3]["bounds"] == [83, 66, 12, 10] and logs[3]["frame"] == 3
                    and 0.99999 <= logs[3]["score"] <= 1, "query result/frame oracle")
            observation = logs[3]["observation"]
            require(observation["frame"] == 3 and observation["attempt"] == 1
                    and observation["epoch"] == 0 and observation["geometry"] == 0
                    and observation["coordinate_space"] == "capture-pixels"
                    and observation["width"] == 1920 and observation["height"] == 1080
                    and logs[-1]["handles_released"] is True, "query result observation/ownership")
            for name in ("id", "process_lifetime", "run", "session"):
                nonempty(observation[name], "query observation " + name)
        waits = [call for call in calls if call[0] == "query_wait"]
        require(len(waits) == 1 and waits[0][3] is not deadline, "single caller wait outcome")
        metrics["query_wait_ns"] = waits[0][2] - waits[0][1]
    else:
        expected = {"observe": 5, "recognize": 10, "release": 14, "log": 6 if kind == "group" else 12}
        require(logs[-1]["searches"] == 10 and logs[-1]["matched"] == 9 and logs[-1]["no_match"] == 1
                and logs[-1]["handles_released"] is True, "replay completion oracle")
        if kind == "group":
            require(stages == ["readiness", "retained-group-enter", "retained-group-settled", "newer-miss", "newer-hit", "complete"],
                    "group markers")
            matches = logs[2]["matched"]
            first = next(index for index, call in enumerate(calls) if call[0] == "recognize")
            group = calls[first:first + 16]
            require([call[0] for call in group] == [name for _ in range(8) for name in ("recognize", "release")],
                    "uninterrupted recognize/release group")
            metrics["group_span_ns"] = group[-1][2] - group[0][1]
            for index in range(8):
                for offset, name in enumerate(("recognize", "release")):
                    call = group[2 * index + offset]
                    metrics[f"{name}_{index}_ns"] = call[2] - call[1]
        else:
            require(stages == ["readiness"] + ["retained-hit"] * 8 + ["newer-miss", "newer-hit", "complete"],
                    "retained replay markers")
            matches = logs[1:9]
        require(len(matches) == 8, "eight retained results")
        for index, found in enumerate(matches):
            require(found["template"] == index and found["bounds"] == [80 + index % 4 * 72, 64 + index // 4 * 72, 12, 10]
                    and 0.99999 <= found["score"] <= 1, "retained result oracle")
        require(logs[-3]["matched"] is False and logs[-3]["frame"] == 3
                and logs[-2]["bounds"] == [83, 66, 12, 10] and logs[-2]["frame"] == 4,
                "new-frame sequence oracles")
    for name, count in expected.items():
        require(counts[name]["count"] == count, "host operation count: " + name)
    require(all(value["failures"] == int(deadline and name == "query_wait") for name, value in counts.items()),
            "host failure boundary")
    for key in ("child", "supervisor"):
        value = report["metrics"].get(key + "_rss_bytes")
        if value is not None:
            metrics[key + "_sampled_rss_bytes"] = integer(value, "sampled RSS", 1)
    children = [mark["pid"] for mark in report["milestones"] if mark["event"] == "ChildStarted"]
    require(len(children) == 1, "one supervised engine child")
    diagnostics = None
    if deadline:
        diagnostics = {
            "requested_wait_ns": 1_000_000,
            "actual_wait_ns": metrics["query_wait_ns"],
            "caller_overrun_ns": max(0, metrics["query_wait_ns"] - 1_000_000),
            "cleanup_us": metrics["cleanup_us"],
            "entry_settlement": [
                {"child_at_us": mark.get("at_us"), "parent_received_us": mark.get("supervisor_received_us")}
                for mark in report["milestones"] if mark["event"] == "EntrySettled"],
            "boundary_note": "child and parent settlement epochs remain separate; no cross-epoch subtraction",
        }
    return {"metrics": metrics, "application_status": report["status"], "primary": primary,
            "selected_pids": children, "driver_declared_outer": None, "deadline_diagnostics": diagnostics}


def cpu_observation(record):
    cpu = record.get("cpu") or {}
    failures = []
    value = None
    if cpu.get("user_seconds") is not None and cpu.get("system_seconds") is not None:
        value = str(exact(cpu["user_seconds"]) + exact(cpu["system_seconds"]))
    if cpu.get("status") != "complete" or cpu.get("final") is not True or value is None:
        failures.append(issue("unavailable", "complete final owned-invocation CPU coverage unavailable"))
    resolution = cpu.get("accounting_resolution_seconds")
    if resolution is None or exact(resolution) <= 0:
        failures.append(issue("unavailable", "effective lifetime CPU accounting resolution unavailable"))
    return value, resolution, failures


def loader_check(plan, output, record, sink_root):
    """Validate retained child transports without calling them image closure."""
    reports = {}

    def visit(value):
        if isinstance(value, dict):
            if value.get("loader_enabled") is True:
                name = value["sink"]["path"]
                check(name not in reports or reports[name] == value, "identity", "conflicting repeated child transport")
                reports[name] = value
            for item in value.values():
                visit(item)
        elif isinstance(value, list):
            for item in value:
                visit(item)

    visit(output)
    if not plan["loader"]:
        require(not reports and record["stderr"] == "", "unexpected loader/stderr output")
        return []
    require(plan["target"] == component.TARGETS[0], "DYLD transport is Apple-only")
    require(len(reports) == 2, "exactly two selected child transports required")
    retained, pids, names = [], set(), set()
    for name, report in reports.items():
        path = Path(name)
        check(path.parent == sink_root and not path.is_symlink(), "identity", "child sink outside owned directory")
        require(report["complete"] is True and report["eof"] is True
                and report.get("read_error") is None and report.get("read_limit_reached") is not True,
                "incomplete child transport")
        require(all(report[key] == 0 for key in ("foreign_lines", "interleaved", "malformed_prefixes", "oversized_lines")),
                "ambiguous child transport")
        require(report["sink"]["truncated"] is False and report["sink"]["write_error"] is None, "child sink retention failed")
        size = path.stat().st_size
        require(size == report["sink"]["bytes_written"] <= plan["bounds"]["child_sink_bytes"], "child sink byte limit")
        payload = path.read_bytes()
        lines = payload.splitlines(keepends=True)
        prefix = f"dyld[{report['child_pid']}]: ".encode()
        require(len(lines) == report["lines"] and lines
                and all(line.startswith(prefix) and line.endswith(b"\n") for line in lines), "owned PID transport lines")
        names.add(path)
        pids.add(report["child_pid"])
        retained.append({"path": str(path), "sha256": hashlib.sha256(payload).hexdigest(),
                         "bytes": size, "pid": report["child_pid"], "scope": "transport-only-not-image-closure"})
    check(len(pids) == 2 and record["process_id"] not in pids, "identity", "distinct selected transport PIDs")
    require(set(sink_root.iterdir()) == names, "unreported child sink")
    prefix = f"dyld[{record['process_id']}]: "
    lines = record["stderr"].splitlines()
    require(lines and all(line.startswith(prefix) for line in lines), "actual root PID loader transport")
    return retained


def terminal_issues(plan, role, output):
    """Inspect sticky application, identity and cleanup facts independently of result validation."""
    failures = []
    if plan["case"].startswith("trial-"):
        if not (output.get("success") is True and output.get("status") == "semantic_pass"
                and output.get("apparatus_failures") == []):
            failures.append(issue("semantics", "public trial operation failed"))
        oracle = output.get("oracle")
        if not (isinstance(oracle, dict) and oracle.get("passed") is True and oracle.get("violations") == []):
            failures.append(issue("semantics", "trial oracle failed"))
        operation = output.get("operation") or {}
        controller = operation.get("controller") or {}
        reports = [controller.get("result"), *controller.get("attempts", [])]
        if operation and (operation.get("shutdown", {}).get("completed") is not True
                          or operation.get("shutdown", {}).get("error") is not None
                          or operation.get("requires_parent_owned_tree_cleanup") is True
                          or (operation.get("accepted") is not False and operation.get("terminal_observed") is not True)):
            failures.append(issue("cleanup", "trial shutdown/terminal unconfirmed"))
    else:
        reports = output.get("runs", [])
    for report in reports:
        if not isinstance(report, dict):
            continue
        cleanup = report.get("cleanup")
        if cleanup is not None and (cleanup.get("clean") is not True or
                                   cleanup.get("session_closed", cleanup.get("engine", {}).get("session_closed")) is not True):
            failures.append(issue("cleanup", "application child cleanup unconfirmed"))
        reaped = report.get("child_reaped", report.get("observations", {}).get("child_reaped"))
        if reaped is False:
            failures.append(issue("cleanup", "application child not reaped"))
        build = report.get("build")
        if build is not None and (build.get("executable_sha256") != role["engine"]["sha256"]
                                  or build.get("target") != plan["target"]):
            failures.append(issue("identity", "terminal child executable/target mismatch"))
    return failures


def reconcile(plan, slot, record, review=None, sink_root=None, runtime_observation=None):
    failures = []
    unavailable = []
    role_name = "baseline" if slot["role"] == "control" else slot["role"]
    role = plan["roles"][role_name]
    expected_exit = int(plan["case"].startswith("deadline-"))
    for condition, category, reason in (
        (record.get("cleanup_ok") is True, "cleanup", "outer owned cleanup unconfirmed"),
        (record.get("launch_error") is None, "execution", "launch failure"),
        (record.get("timed_out") is False, "execution", "outer deadline exceeded"),
        (record.get("output_limited") is False, "execution", "combined output bound exceeded"),
        (record.get("exit_code") == expected_exit, "execution", "unexpected original exit status"),
    ):
        if not condition:
            failures.append(issue(category, reason))
    details = {"metrics": {}, "application_status": None, "primary": None, "selected_pids": [], "driver_declared_outer": None}
    output = None
    try:
        output = strict_json(record["stdout"])
        require(isinstance(output, dict), "consumer JSON object required")
        # Preserve the original failure before checking cleanup or a different oracle.
        if plan["case"].startswith("trial-"):
            details["application_status"] = output.get("status")
        elif output.get("runs"):
            details.update(application_status=output["runs"][0].get("status"), primary=output["runs"][0].get("primary"))
        failures.extend(terminal_issues(plan, role, output))
        checked = (trial_check(plan, role, output, details["metrics"]) if plan["case"].startswith("trial-")
                   else replay_check(plan, role, output))
        details.update(checked)
        details["sinks"] = loader_check(plan, output, record, sink_root)
        if record.get("process_id") in details["selected_pids"]:
            failures.append(issue("identity", "root and selected child PID overlap"))
    except (KeyError, IndexError, TypeError, ValueError, OSError, StopIteration) as error:
        failures.append(issue(getattr(error, "category", "semantics"), str(error)))
    if details["driver_declared_outer"] is not None:
        embedded = details["driver_declared_outer"]
        actual = plan["bounds"]
        if (embedded.get("outer_primary_ms", math.inf) > actual["primary_seconds"] * 1000
                or embedded.get("outer_cleanup_ms") != actual["cleanup_seconds"] * 1000
                or embedded.get("outer_output_bytes", math.inf) > actual["output_limit_bytes"]):
            failures.append(issue("execution", "invoking envelope is smaller than driver-declared outer contract"))
    value, resolution, margin = None, None, None
    cpu_issues = []
    coverage = {"source_closed_children": {"eligible": False, "status": "unavailable"}}
    try:
        value, resolution, cpu_issues = cpu_observation(record)
        details["raw_cpu_observation"] = {"user_plus_system_seconds": value,
                                         "status": (record.get("cpu") or {}).get("status"),
                                         "scope": (record.get("cpu") or {}).get("scope"),
                                         "reason": (record.get("cpu") or {}).get("reason")}
        details["metrics"]["invocation_elapsed_seconds"] = str(exact(record["duration_seconds"]))
    except (KeyError, TypeError, ValueError) as error:
        cpu_issues.append(issue("unavailable", "CPU/elapsed: " + str(error)))
    try:
        bundle = runtime_bundle(plan)
        require(runtime_observation is not None, "actual invocation pre/post runtime bindings missing")
        for problem in runtime_observation.get("issues", []):
            (failures if problem["category"] == "identity" else unavailable).append(problem)
        details["runtime_images"] = runtime.observe(plan, bundle, role_name, record, output, sink_root, runtime_observation)
        certificate, margin = accounting_method(plan, review, bundle)
        require(not failures, "failed semantic/cleanup path cannot establish source-closed CPU eligibility")
        coverage["source_closed_children"] = runtime.terminal_witness(plan, role_name, certificate, record, output, details["runtime_images"])
        coverage["source_closed_children"]["certificate"] = review["source_closed_cpu"]
        details["metrics"]["lifetime_cpu_seconds"] = value
        details["accounting_allowance_seconds"] = margin
        details["accounting_allowance"] = review["accounting_allowance"]
    except (KeyError, IndexError, TypeError, ValueError, OSError, runtime.images._Refusal) as error:
        failure = issue(getattr(error, "category", "unavailable"), "source-closed CPU/image coverage: " + str(error))
        (failures if failure["category"] == "identity" else unavailable).append(failure)
        unavailable.extend(cpu_issues)
        unavailable.append(issue("unavailable", "source-closed owned roles unavailable; no complete identified census inferred"))
    details["cpu_coverage"] = coverage
    for metric, contract in (review or {}).get("metrics", {}).items():
        point = details["metrics"].get(metric)
        if point is None:
            unavailable.append(issue("unavailable", "required metric missing: " + metric))
        elif isinstance(contract, dict) and contract.get("unit") == "bytes":
            # Every invocation, calibration included, obeys the declared point ceiling.
            try:
                ceiling, _ = resident_bounds(contract)
            except ValueError as error:
                unavailable.append(issue("unavailable", metric + ": " + str(error)))
            else:
                if exact(point) > ceiling:
                    failures.append(issue("numerical", metric + " exceeds fixed resident checkpoint ceiling"))
        elif trial_metric(plan["case"], metric):
            # Every trial operation, smoke with a review and calibration included, obeys the declared ceiling.
            try:
                ceiling = trial_budget(contract)["hard_ceiling_us"]
            except ValueError as error:
                unavailable.append(issue("unavailable", metric + ": " + str(error)))
            else:
                if exact(point) > ceiling:
                    failures.append(issue("numerical", metric + " exceeds fixed trial responsiveness ceiling"))
        if metric == "lifetime_cpu_seconds" and resolution is not None and "resolution" in contract:
            if exact(resolution) > exact(contract["resolution"]):
                failures.append(issue("clock", "CPU resolution coarser than frozen method"))
    for metric in required_metrics(plan["case"]):
        details["metrics"].setdefault(metric, None)
    details["ungraded_resource_domains"] = {
        "logical_bgr": "unavailable: retained normal consumer exports no logical cache counters",
        "requested_rust_bytes": "unavailable: normal consumer has no counting allocator",
        "native_allocations": "unavailable: RSS is not an allocation-domain measurement",
        "os_high_water": "unavailable: process points/sampled maxima are not OS high-water",
        "same_owner_growth": "unavailable: no matched same-owner before/after pair",
        "final_owner_residual": "unavailable: post-owner whole-process RSS is not owner residual",
    }
    return {**details, "issues": failures, "unavailable": unavailable,
            "cpu": record.get("cpu"), "process_accounting": record.get("process_accounting"),
            "original_exit_code": record.get("exit_code"), "cpu_resolution": resolution}


def scalar_envelope(values, quantum, resolution):
    require(len(values) == 6, "six calibration invocations required")
    values = [exact(value) for value in values]
    q, r = exact(quantum), exact(resolution)
    require(q > 0 and r > 0, "positive reporting quantum and resolution required")
    spread = max(values) - min(values)
    require(spread / max(median(values), 2 * r) <= Fraction(15, 100), "calibration noise exceeds 0.15")
    allowance = max(spread, 2 * r)
    ticks = (max(values) + allowance) / q
    ceiling = q * (-(-ticks.numerator // ticks.denominator))
    return {"ceiling": str(ceiling), "allowance": str(allowance), "resolution": str(r), "quantum": str(q),
            "spread": str(spread), "median": str(median(values)), "minimum": str(min(values)), "maximum": str(max(values))}


def resident_envelope(values, ceiling, floor):
    """Declared point ceiling plus empirical comparison allowance; no peak, growth or residual."""
    require(len(values) == 6 and all(type(value) is int and 0 < value <= ceiling for value in values),
            "six positive integral resident bytes within the fixed checkpoint ceiling required")
    center, spread = median(values), max(values) - min(values)
    # Same 15% method tolerance as timing, reported as repeatability, never confidence.
    require(spread <= center * Fraction(15, 100), "calibration resident repeatability exceeds 0.15")
    return {"resource": True, "ceiling": str(ceiling), "comparison_allowance_floor_bytes": str(floor),
            "allowance": str(max(spread, floor)), "spread": str(spread), "median": str(center),
            "minimum": str(min(values)), "maximum": str(max(values))}


def trial_central_four(values):
    """One occurrence per tail; indexes identify the unchanged input order."""
    require(len(values) == 6, "six observations required for central-four repeatability")
    ranked = sorted(enumerate(exact(value) for value in values), key=lambda item: (item[1], item[0]))
    retained = ranked[1:-1]
    points = [value for _, value in retained]
    return {
        "input_indices": [index for index, _ in retained],
        "excluded": [{"side": side, "input_index": index, "value": str(value)}
                     for side, (index, value) in zip(("minimum", "maximum"), (ranked[0], ranked[-1]))],
        "values": [str(value) for value in points],
        "minimum": str(points[0]), "maximum": str(points[-1]),
        "spread": str(points[-1] - points[0]), "mean": str(sum(points) / 4), "median": str(median(points)),
    }


def trial_envelope(values, contract):
    """Fixed ceiling/tolerance; central-four repeatability never discards an invocation."""
    budget, (quantum, resolution) = trial_contract(contract)
    require(len(values) == 6, "six calibration invocations required")
    values = [exact(value) for value in values]
    require(max(values) <= budget["hard_ceiling_us"], "calibration operation exceeds fixed trial responsiveness ceiling")
    spread = max(values) - min(values)
    central = trial_central_four(values)
    check(exact(central["spread"]) <= budget["max_regression_us"], "inconclusive",
          "calibration central-four spread exceeds max_regression_us")
    return {**budget, "ceiling": str(budget["hard_ceiling_us"]), "allowance": str(budget["max_regression_us"]),
            "quantum": str(quantum), "resolution": str(resolution), "spread": str(spread),
            "median": str(median(values)), "minimum": str(min(values)), "maximum": str(max(values)),
            "central_four": central}


def derive_envelopes(rows, review, case):
    require(len(rows) == 6, "complete six-invocation calibration required")
    result = {}
    for name, contract in review["metrics"].items():
        values = [row["metrics"].get(name) for row in rows]
        require(all(value is not None for value in values), "unavailable calibration: " + name)
        if trial_metric(case, name):
            result[name] = trial_envelope(values, contract)
        elif contract.get("unit") == "bytes":
            require(contract.get("domain") == "resident", "normal consumer supports resident endpoints only")
            result[name] = resident_envelope(values, *resident_bounds(contract))
        elif name == "lifetime_cpu_seconds" and contract.get("accounting_allowance") is not None:
            margins = [row.get("accounting_allowance_seconds") for row in rows]
            require(all(value is not None for value in margins) and len(set(margins)) == 1,
                    "source/host/API accounting allowance missing or changed")
            require(all(row.get("accounting_allowance") == contract["accounting_allowance"] for row in rows),
                    "CPU allowance evidence changed")
            envelope = scalar_envelope(values, contract["quantum"], margins[0])
            envelope["accounting_allowance_seconds"] = envelope.pop("resolution")
            envelope["margin_kind"] = "prospective-accounting-allowance"
            envelope["accounting_allowance"] = contract["accounting_allowance"]
            result[name] = envelope
        else:
            resolutions = [exact(contract["resolution"])]
            if name == "lifetime_cpu_seconds":
                require(all(row.get("cpu_resolution") is not None for row in rows), "CPU resolution unavailable")
                resolutions.extend(exact(row["cpu_resolution"]) for row in rows)
            result[name] = scalar_envelope(values, contract["quantum"], max(resolutions))
    return result


def invocation_limits(row, envelopes):
    failures = []
    for name, envelope in envelopes.items():
        value = row["metrics"].get(name)
        if value is None:
            failures.append(issue("unavailable", row["slot"]["id"] + ": " + name))
        elif exact(value) > exact(envelope["ceiling"]):
            failures.append(issue("numerical", row["slot"]["id"] + ": " + name + " exceeds frozen ceiling"))
        if name == "lifetime_cpu_seconds" and not envelope.get("resource"):
            if "accounting_allowance_seconds" in envelope:
                if (row.get("accounting_allowance") != envelope["accounting_allowance"]
                        or row.get("accounting_allowance_seconds") != envelope["accounting_allowance_seconds"]):
                    failures.append(issue("clock", "CPU accounting allowance cannot drift after freeze"))
            else:
                resolution = row.get("cpu_resolution")
                if resolution is None:
                    failures.append(issue("unavailable", "CPU resolution unavailable"))
                elif exact(resolution) > exact(envelope["resolution"]):
                    failures.append(issue("clock", "CPU clock cannot enlarge frozen allowance"))
    return failures


def trial_findings(rows, stage, roles, name, envelope):
    """Six balanced pairs at one fixed absolute tolerance; finite-cohort checks, not intervals or population claims."""
    tolerance = exact(envelope["max_regression_us"])
    failures, medians, pairs = [], {}, {}
    for role in roles:
        selected = [row for row in rows if row["slot"]["role"] == role]
        pairs[role] = {row["slot"]["pair"]: exact(row["metrics"][name])
                       for row in selected if row["metrics"].get(name) is not None}
        if len(selected) != 6 or any(row["metrics"].get(name) is None for row in selected):
            continue
        values = [exact(row["metrics"][name]) for row in selected]
        if exact(trial_central_four(values)["spread"]) > tolerance:
            failures.append(issue("inconclusive", f"{role}: {name} central-four range exceeds max_regression_us"))
        first = [row["metrics"][name] for row in selected if row["slot"]["position"] == 0]
        second = [row["metrics"][name] for row in selected if row["slot"]["position"] == 1]
        if len(first) != 3 or len(second) != 3 or abs(median(first) - median(second)) > tolerance:
            failures.append(issue("inconclusive", f"{role}: {name} order exceeds max_regression_us"))
        chronological = [row["metrics"][name] for row in sorted(selected, key=lambda row: row["slot"]["process"])]
        if abs(median(chronological[:3]) - median(chronological[3:])) > tolerance:
            failures.append(issue("inconclusive", f"{role}: {name} drift exceeds max_regression_us"))
        medians[role] = median(values)
    baseline, other = roles
    differences = [(pair, pairs[other][pair] - pairs[baseline][pair])
                   for pair in sorted(set(pairs[baseline]) & set(pairs[other]))]
    center = medians[other] - medians[baseline] if len(medians) == 2 else None
    if stage == "control":
        for pair, difference in differences:
            if abs(difference) > tolerance:
                failures.append(issue("inconclusive", f"{name}: unchanged A/A pair {pair} difference exceeds max_regression_us"))
        if center is not None and abs(center) > tolerance:
            failures.append(issue("inconclusive", f"{name}: unchanged A/A role median difference exceeds max_regression_us"))
        return failures
    # Observed non-regression budget only: not a causal SDK regression; faster needs no minimum gain.
    for pair, difference in differences:
        if difference > tolerance:
            failures.append(issue("numerical", f"{name}: pair {pair} candidate exceeds baseline plus max_regression_us"))
    if center is not None and center > tolerance:
        failures.append(issue("numerical", f"{name}: candidate median exceeds baseline median plus max_regression_us"))
    return failures


def grade_rows(rows, stage, envelopes, case):
    failures = []
    expected = schedule(stage)
    if [row["slot"] for row in rows] != expected:
        failures.append(issue("incomplete", "complete original balanced schedule required"))
    for row in rows:
        failures.extend(invocation_limits(row, envelopes))
    if stage not in ("control", "comparison"):
        return failures
    roles = ("baseline", "control" if stage == "control" else "candidate")
    for name, envelope in envelopes.items():
        if trial_metric(case, name):
            failures.extend(trial_findings(rows, stage, roles, name, envelope))
            continue
        resource = envelope.get("resource")
        medians = {}
        for role in roles:
            selected = [row for row in rows if row["slot"]["role"] == role]
            if len(selected) != 6 or any(row["metrics"].get(name) is None for row in selected):
                continue
            values = [exact(row["metrics"][name]) for row in selected]
            medians[role] = median(values)
            allowance = exact(envelope["allowance"])
            if resource:
                noisy = max(values) - min(values) > median(values) * Fraction(15, 100)
            else:
                r = exact(envelope.get("accounting_allowance_seconds", envelope.get("resolution")))
                noisy = (max(values) - min(values)) / max(median(values), 2 * r) > Fraction(15, 100)
            if noisy:
                failures.append(issue("numerical", role + ": " + name + (" repeatability" if resource else " noise")))
            first = [row["metrics"][name] for row in selected if row["slot"]["position"] == 0]
            second = [row["metrics"][name] for row in selected if row["slot"]["position"] == 1]
            if len(first) != 3 or len(second) != 3 or abs(median(first) - median(second)) > allowance:
                failures.append(issue("numerical", role + ": " + name + " order"))
            chronological = sorted(selected, key=lambda row: row["slot"]["process"])
            if abs(median([row["metrics"][name] for row in chronological[:3]]) -
                   median([row["metrics"][name] for row in chronological[3:]])) > allowance:
                failures.append(issue("numerical", role + ": " + name + " drift"))
        if len(medians) == 2:
            difference = medians[roles[0]] - medians[roles[1]]
            if stage == "control" and abs(difference) > exact(envelope["allowance"]):
                failures.append(issue("numerical", name + ": unchanged A/A role bias"))
            if stage == "comparison" and name == benefit_metric(case) and difference <= exact(envelope["allowance"]):
                failures.append(issue("numerical", name + ": scalar benefit does not exceed allowance"))
            # Non-regression only: never a benefit claim and never a substitute for one.
            if stage == "comparison" and resource and -difference > exact(envelope["allowance"]):
                failures.append(issue("numerical", name + ": candidate resident median exceeds baseline plus allowance"))
    return failures


def cohort_statistics(rows, case, *, central_four=False):
    result = {}
    for role in sorted({row["slot"]["role"] for row in rows}):
        selected = [row for row in rows if row["slot"]["role"] == role]
        metrics = sorted({name for row in selected for name in row["metrics"]})
        result[role] = {}
        for name in metrics:
            values = [row["metrics"].get(name) for row in selected]
            present = [exact(value) for value in values if value is not None]
            result[role][name] = {
                "values": [None if value is None else str(exact(value)) for value in values],
                "count": len(present), "missing": len(values) - len(present),
                "median": str(median(present)) if present else None,
                "minimum": str(min(present)) if present else None,
                "maximum": str(max(present)) if present else None,
            }
            if trial_metric(case, name):
                result[role][name].update(
                    slots=[row["slot"] for row in selected],
                    mean=str(sum(present) / len(present)) if present else None,
                    spread=str(max(present) - min(present)) if present else None,
                )
                if central_four:
                    result[role][name]["central_four"] = trial_central_four(values) if len(values) == len(present) == 6 else None
    return result


def admission_room(now, deadline, attempts, bounds):
    # Reserve a complete pair before its first launch, not merely the first child.
    per_attempt = LIMITS["primary_seconds"] + LIMITS["cleanup_seconds"] + LIMITS["retention_seconds"]
    return now + attempts * per_attempt + LIMITS["final_retention_seconds"] <= deadline


def observation_findings(issues):
    return [problem for problem in issues if problem["category"] in ("numerical", "clock")]


def phase_outcomes(summary):
    qualification_only = {"numerical", "inconclusive", "clock", "unavailable", "admission", "prerequisite", "incomplete"}
    execution = [problem for problem in summary["issues"] if problem["category"] not in qualification_only]
    incomplete = summary["unrun"] or any(problem["category"] == "incomplete" for problem in summary["issues"])
    return {
        "execution_status": "failed" if execution else "incomplete" if incomplete else "completed",
        "qualification_status": ("not-requested" if summary["stage"] == "smoke" else
                                 "failed" if summary["issues"] else "passed"),
    }


def numerical_control_only(summary, entry):
    """Admit diagnostics from sealed original facts, never by regrading a control."""
    categories = {problem["category"] for problem in summary["issues"]}
    if (summary.get("format") not in ("consumer-phase-v1", "consumer-phase-v2")
            or summary.get("stage") != "control" or summary.get("admission_issues") != []
            or not categories & {"numerical", "inconclusive"}
            or categories - {"numerical", "inconclusive", "incomplete"}):
        return False
    require(summary["plan_sha256"] == entry["plan_sha256"], "control registration plan changed")
    tasks = schedule("control")
    attempts = summary["attempts"]
    require(attempts and summary["schedule"] == tasks
            and summary["unrun"] == tasks[len(attempts):], "control schedule lineage changed")
    directory = Path(entry["output"])
    for index, attempt in enumerate(attempts):
        slot = tasks[index]
        require(attempt["slot"] == slot, "control attempt slot changed")
        receipts = {}
        for kind, maximum in (("started", 64 * 1024), ("raw", 25 * MIB), ("validation", 256 * 1024)):
            binding = attempt.get(kind)
            require(isinstance(binding, dict) and binding["name"] == slot["id"] + "." + kind + ".json",
                    "control attempt has unresolved " + kind)
            receipts[kind] = read_reference(
                {"path": str(directory / binding["name"]), "sha256": binding["sha256"]}, maximum)
            require(receipts[kind]["slot"] == slot, "control receipt slot changed")
        require(all(receipts[kind]["plan_sha256"] == summary["plan_sha256"] for kind in ("started", "raw")),
                "control receipt plan changed")
        record = receipts["raw"]["record"]
        require(record.get("cleanup_ok") is True and record.get("launch_error") is None
                and record.get("timed_out") is False and record.get("output_limited") is False,
                "control execution or cleanup unresolved")
        validation = receipts["validation"]
        require(type(record.get("exit_code")) is int
                and record["exit_code"] == int(summary["case"].startswith("deadline-"))
                and record["exit_code"] == validation["original_exit_code"],
                "control terminal status changed")
        if any(problem["category"] not in ("numerical", "inconclusive") for problem in validation["issues"]):
            return False
    return True


class Campaign:
    def __init__(self, plan):
        self.root = Path(plan["campaign"])
        self.plan = plan

    def initialize(self):
        self.root.mkdir(parents=True, exist_ok=True)
        for name in ("phases", "fences"):
            (self.root / name).mkdir(exist_ok=True)
        binding = {"format": "consumer-campaign-v1", "host": self.plan["host"], "target": self.plan["target"]}
        path = self.root / "campaign.json"
        if path.exists():
            require(load(path) == binding, "campaign target/host changed")
        else:
            save(path, binding)

    def fences(self, *, ignore_phase=None, diagnostic=False):
        failures, numerical_controls = [], set()
        for path in sorted((self.root / "phases").glob("*.json")):
            if path.stem == ignore_phase:
                continue
            entry = load(path)
            summary_name = entry.get("summary_name", "summary.json")
            require(Path(summary_name).name == summary_name, "invalid registered summary path")
            summary = Path(entry["output"]) / summary_name
            completion = self.root / "phases" / (path.stem + ".complete")
            if not completion.exists():
                failures.append(issue("cleanup", "unfinished phase registration: " + path.name))
            else:
                try:
                    seal = load(completion)
                    require(seal["path"] == str(summary), "phase summary path changed")
                    prior = read_reference(seal)
                    numerical = diagnostic and entry["stage"] == "control" and numerical_control_only(prior, entry)
                    if numerical:
                        numerical_controls.add(path.stem)
                    for failure in prior["issues"]:
                        if failure["category"] in ("identity", "cleanup") or (entry["stage"] == "control" and not numerical):
                            failures.append(failure)
                except (KeyError, IndexError, TypeError, ValueError, OSError) as error:
                    failures.append(issue("identity", "prior receipt: " + str(error)))
        for path in sorted((self.root / "fences").glob("*.json")):
            entry = load(path)
            if diagnostic and entry.get("category") == "control" and entry.get("phase") in numerical_controls:
                continue
            failures.append({**entry, "fence": reference(path)})
        return failures

    def fence(self, category, reason, phase):
        entry = {"category": category, "reason": reason, "phase": phase,
                 "context_sha256": context_digest(self.plan)}
        path = self.root / "fences" / (hashed(entry) + ".json")
        if not path.exists():
            save(path, entry, 64 * 1024)

    @contextlib.contextmanager
    def locked(self):
        self.initialize()
        path = self.root / "active"
        try:
            save(path, {"pid": os.getpid(), "host": self.plan["host"]}, 64 * 1024)
        except FileExistsError:
            yield [issue("admission", "campaign has an active or interrupted phase owner; no resume")]
            return
        try:
            yield []
        finally:
            path.unlink()


def load_cohort(directory):
    directory = Path(directory).resolve(strict=True)
    require((directory / "plan.json").stat().st_size <= 4 * MIB, "phase plan exceeds retained bound")
    plan_bytes = (directory / "plan.json").read_bytes()
    require(len(plan_bytes) <= 4 * MIB, "phase plan exceeds retained bound")
    plan = strict_json(plan_bytes.decode("utf-8"))
    validate_plan(plan)
    summary_path = directory / "summary.json"
    require(summary_path.stat().st_size <= 4 * MIB, "phase summary exceeds retained bound")
    summary_bytes = summary_path.read_bytes()
    require(len(summary_bytes) <= 4 * MIB, "phase summary exceeds retained bound")
    summary = strict_json(summary_bytes.decode("utf-8"))
    summary_binding = {"path": str(summary_path), "sha256": hashlib.sha256(summary_bytes).hexdigest()}
    require(summary["format"] in ("consumer-phase-v1", "consumer-phase-v2")
            and summary["plan_sha256"] == hashlib.sha256(plan_bytes).hexdigest(), "phase plan binding changed")
    require(summary["schedule"] == schedule(summary["stage"]), "phase schedule changed")
    review, admission_issues = admission(plan)
    rows, failures = [], list(summary["issues"])
    try:
        phase = plan["case"] + "-" + summary["stage"]
        if summary["format"] == "consumer-phase-v2" and summary["stage"] == "smoke":
            phase += "-" + hashed(str(directory))
        registration = Path(plan["campaign"]) / "phases" / (phase + ".json")
        require(summary.get("registration") == reference(registration), "phase registration binding changed")
        registered = read_reference(summary["registration"])
        require(registered == {"output": str(directory), "stage": summary["stage"],
                               "plan_sha256": summary["plan_sha256"]}, "phase registration lineage changed")
        seal = load(registration.with_suffix(".complete"))
        require(seal["path"] == summary_binding["path"] and seal["sha256"] == summary_binding["sha256"],
                "phase completion seal changed")
    except (KeyError, ValueError, OSError) as error:
        failures.append(issue("identity", "cohort lineage: " + str(error)))
    if summary["stage"] != "smoke":
        failures.extend(admission_issues)
    expected = schedule(summary["stage"])
    for index, attempt in enumerate(summary["attempts"]):
        require(attempt["slot"] == expected[index], "attempt order changed")
        start_path = directory / attempt["started"]["name"]
        require(start_path.parent == directory, "started receipt path changed")
        start = read_reference({"path": str(start_path), "sha256": attempt["started"]["sha256"]}, 64 * 1024)
        require(start["slot"] == attempt["slot"] and start["plan_sha256"] == summary["plan_sha256"],
                "started receipt slot binding changed")
        if attempt.get("raw") is None:
            failures.append(issue("incomplete", "started attempt has no terminal raw receipt"))
            continue
        binding = attempt["raw"]
        path = directory / binding["name"]
        require(path.parent == directory, "raw receipt path changed")
        raw = read_reference({"path": str(path), "sha256": binding["sha256"]}, 25 * MIB)
        require(raw["slot"] == attempt["slot"] and raw["plan_sha256"] == summary["plan_sha256"], "raw slot binding changed")
        require(raw["execution_limits"] == {
            "timeout_seconds": plan["bounds"]["primary_seconds"],
            "cleanup_seconds": plan["bounds"]["cleanup_seconds"],
            "output_limit_bytes": plan["bounds"]["output_limit_bytes"]}, "retained execution envelope changed")
        runtime_observation = None
        if attempt.get("runtime") is not None:
            runtime_path = directory / attempt["runtime"]["name"]
            require(runtime_path.parent == directory, "runtime observation path changed")
            runtime_observation = read_reference(
                {"path": str(runtime_path), "sha256": attempt["runtime"]["sha256"]}, 2 * MIB)
            require(runtime_observation["slot"] == raw["slot"] and
                    runtime_observation["plan_sha256"] == summary["plan_sha256"], "runtime observation slot changed")
        result = reconcile(plan, raw["slot"], raw["record"], review, directory / (raw["slot"]["id"] + "-sinks"), runtime_observation)
        if attempt.get("validation") is not None:
            saved = directory / attempt["validation"]["name"]
            require(saved.parent == directory, "validation receipt path changed")
            validation = read_reference({"path": str(saved), "sha256": attempt["validation"]["sha256"]}, 256 * 1024)
            require(validation == {"slot": raw["slot"], **result}, "retained raw/validation disagree")
        rows.append({"slot": raw["slot"], **result})
        descriptive = (observation_findings(result["issues"])
                       if summary["format"] == "consumer-phase-v2" and summary["stage"] == "smoke" else [])
        failures.extend(problem for problem in result["issues"] if problem not in descriptive)
        if summary["stage"] != "smoke":
            failures.extend(result["unavailable"])
    if summary["unrun"] != expected[len(summary["attempts"]):] or len(rows) != len(expected):
        failures.append(issue("incomplete", "unrun or incomplete original slots"))
    return {"plan": plan, "summary": summary, "rows": rows, "issues": failures,
            "review": review, "binding": summary_binding, "directory": str(directory)}


def compatible(first, second):
    require(context_digest(first["plan"]) == context_digest(second["plan"]), "cohort source/method/host context differs")
    require(first["plan"].get("admission") == second["plan"].get("admission"), "cohort method review changed")
    require(first["directory"] != second["directory"], "independent cohort required")
    old = {row["raw"]["sha256"] for row in first["summary"]["attempts"] if row.get("raw")}
    require(not old.intersection(row["raw"]["sha256"] for row in second["summary"]["attempts"] if row.get("raw")),
            "calibration/control raw samples reused")


def accepted_profile(proposal, profile, review):
    require(set(profile) == set(proposal) | {"proposal_sha256", "acceptance"}
            and all(profile[key] == value for key, value in proposal.items()),
            "profile proposal contents changed")
    digest = hashed(proposal)
    require(profile["proposal_sha256"] == digest, "profile proposal digest mismatch")
    require(review.get("accepted") is True and review.get("profile_proposal_sha256") == digest,
            "explicit numerical profile acceptance required")
    nonempty(review.get("reviewer"), "reviewer")
    nonempty(review.get("rationale"), "rationale")
    return proposal["envelopes"]


def prerequisites(plan, stage):
    envelopes = {}
    failures = []
    try:
        if stage == "control":
            calibration = load_cohort(plan["calibration"]["directory"])
            require(calibration["binding"] == plan["calibration"]["binding"], "calibration receipt binding changed")
            require(calibration["summary"]["stage"] == "calibration", "fresh calibration stage required; development distributions cannot qualify")
            require(context_digest(plan) == context_digest(calibration["plan"])
                    and plan.get("admission") == calibration["plan"].get("admission"), "calibration context changed")
            require(not calibration["issues"], "calibration failed or unavailable")
            envelopes = derive_envelopes(calibration["rows"], calibration["review"], plan["case"])
        elif stage == "comparison":
            profile = read_reference(plan["profile"])
            require(profile["format"] == "consumer-profile-v1" and profile["context_sha256"] == context_digest(plan),
                    "frozen profile context changed")
            require(profile["admission"] == plan.get("admission"), "frozen method changed")
            calibration = load_cohort(profile["calibration"]["directory"])
            control = load_cohort(profile["control"]["directory"])
            require(calibration["binding"] == profile["calibration"]["binding"] and
                    control["binding"] == profile["control"]["binding"], "profile receipt binding changed")
            proposal = profile_proposal(calibration, control, check_fences=False)
            review = read_reference(profile["acceptance"])
            envelopes = accepted_profile(proposal, profile, review)
            require(not Campaign(plan).fences(ignore_phase=plan["case"] + "-comparison"),
                    "campaign identity/cleanup/control fence")
    except (KeyError, TypeError, ValueError, OSError) as error:
        failures.append(issue(getattr(error, "category", "prerequisite"), str(error)))
    return envelopes, failures


def collect_steps(plan, stage, output):
    """One live iterator; caller inspection time consumes the same absolute deadline."""
    started = time.monotonic()
    validate_plan(plan)
    require(stage in STAGES, "unknown phase")
    deadline = started + TOTAL_SECONDS[stage]
    if stage == "comparison":
        require("candidate" in plan["roles"], "explicit candidate source/binary required")
    output = Path(output).resolve()
    output.mkdir(parents=False, exist_ok=False)
    plan_ref = save(output / "plan.json", plan)
    campaign = Campaign(plan)
    tasks = schedule(stage)
    summary = {"format": "consumer-phase-v2", "stage": stage, "case": plan["case"],
               "plan_sha256": plan_ref["sha256"], "schedule": tasks, "attempts": [], "unrun": tasks.copy(),
               "issues": [], "qualification": False, "admission_issues": [], "envelopes": {},
               "observation_issues": [], "elapsed_seconds": None, "evidence_bytes": plan_ref["bytes"]}
    rows = []
    phase = plan["case"] + "-" + stage
    if stage == "smoke":
        phase += "-" + hashed(str(output))
    registration = None
    active_runtime = None
    review = None
    with campaign.locked() as lock_issues:
        try:
            review, missing = admission(plan)
            if stage != "smoke" and "candidate" not in plan["roles"]:
                missing.append(issue("unavailable", "candidate source/binary must be frozen before calibration"))
            summary["admission_issues"] = missing
            summary["issues"].extend(lock_issues)
            if not lock_issues:
                summary["issues"].extend(campaign.fences(diagnostic=stage == "smoke"))
            if summary["issues"]:
                return summary
            registration_path = campaign.root / "phases" / (phase + ".json")
            registered = save(registration_path, {"output": str(output), "stage": stage,
                                                "plan_sha256": plan_ref["sha256"]}, 64 * 1024)
            summary["registration"] = {key: registered[key] for key in ("path", "sha256")}
            registration = registration_path
            if stage != "smoke":
                summary["issues"].extend(missing)
            envelopes, missing = prerequisites(plan, stage)
            summary["envelopes"] = envelopes
            summary["issues"].extend(missing)
            summary["issues"].extend(verify_inputs(plan, min(deadline, started + 60)))
            if time.monotonic() > started + LIMITS["admission_seconds"]:
                summary["issues"].append(issue("deadline", "initial admission reservation exceeded"))
            reservation = (48 * len(tasks) + 4) * MIB
            if shutil.disk_usage(output).free < reservation + 64 * MIB:
                summary["issues"].append(issue("storage", "full phase allocation plus 64MiB safety margin unavailable"))
            if summary["issues"]:
                return summary
            for offset in range(0, len(tasks), 2):
                block = tasks[offset:offset + 2]
                if not admission_room(time.monotonic(), deadline, len(block), plan["bounds"]):
                    summary["issues"].append(issue("deadline", "whole-phase complete-block reservation unavailable"))
                    break
                for slot in block:
                    role_name = "baseline" if slot["role"] == "control" else slot["role"]
                    role = plan["roles"][role_name]
                    binding_start = time.monotonic()
                    bundle, observed = None, None
                    if plan.get("runtime_closure") is not None:
                        bundle = runtime_bundle(plan)
                        pre = capture_runtime(plan, bundle, role_name, min(deadline, binding_start + plan["bounds"]["retention_seconds"]))
                        observed = {"slot": slot, "plan_sha256": plan_ref["sha256"],
                                    "closure_sha256": runtime.images.canonical_sha256(bundle),
                                    "pre": pre["value"], "post": None, "issues": pre["issues"]}
                        if pre["issues"]:
                            snapshot_ref = save(output / (slot["id"] + ".unlaunched-runtime.json"), observed, 2 * MIB)
                            summary.setdefault("unlaunched_runtime", []).append(snapshot_ref)
                            summary["issues"].extend(pre["issues"])
                            break
                        active_runtime = (bundle, role_name, observed)
                    env = dict(plan["env"])
                    sinks = output / (slot["id"] + "-sinks")
                    if plan["loader"]:
                        sinks.mkdir()
                        env["MADO_P050_STDERR_SINK_DIR"] = str(sinks)
                    attempt = {"slot": slot, "raw": None, "validation": None, "state": "started"}
                    started_ref = save(output / (slot["id"] + ".started.json"),
                         {"slot": slot, "plan_sha256": plan_ref["sha256"], "argv": role["argv"],
                          "env": env, "elapsed_seconds": time.monotonic() - started,
                          "launch_disposition": "unknown", "cleanup_ok": None}, 64 * 1024)
                    attempt["started"] = {"name": slot["id"] + ".started.json", "sha256": started_ref["sha256"]}
                    summary["evidence_bytes"] += started_ref["bytes"]
                    summary["attempts"].append(attempt)
                    summary["unrun"] = tasks[len(summary["attempts"]):]
                    binding_elapsed = time.monotonic() - binding_start
                    if binding_elapsed > plan["bounds"]["retention_seconds"] or time.monotonic() >= deadline:
                        attempt["state"] = "not_launched"
                        attempt["not_launched"] = "prelaunch retention/whole deadline exhausted"
                        summary["issues"].append(issue("deadline", attempt["not_launched"]))
                        break
                    record = run_process(role["argv"], cwd=Path(plan["root"]), env=env,
                                         timeout_seconds=plan["bounds"]["primary_seconds"],
                                         output_limit_bytes=plan["bounds"]["output_limit_bytes"],
                                         cleanup_seconds=plan["bounds"]["cleanup_seconds"])
                    retention_start = time.monotonic()
                    retention_deadline = min(deadline, retention_start + plan["bounds"]["retention_seconds"] - binding_elapsed)
                    raw_path = output / (slot["id"] + ".raw.json")
                    raw_ref = save(raw_path, {"slot": slot, "plan_sha256": plan_ref["sha256"],
                                             "execution_limits": {
                                                 "timeout_seconds": plan["bounds"]["primary_seconds"],
                                                 "cleanup_seconds": plan["bounds"]["cleanup_seconds"],
                                                 "output_limit_bytes": plan["bounds"]["output_limit_bytes"]},
                                             "record": record}, 25 * MIB)
                    attempt["raw"] = {"name": raw_path.name, "sha256": raw_ref["sha256"]}
                    if observed is not None:
                        post = capture_runtime(plan, bundle, role_name, retention_deadline)
                        observed["post"] = post["value"]
                        observed["post_checked"] = True
                        observed["issues"].extend(post["issues"])
                        runtime_path = output / (slot["id"] + ".runtime.json")
                        runtime_ref = save(runtime_path, observed, 2 * MIB)
                        attempt["runtime"] = {"name": runtime_path.name, "sha256": runtime_ref["sha256"]}
                        summary["evidence_bytes"] += runtime_ref["bytes"]
                        active_runtime = None
                    result = reconcile(plan, slot, record, review, sinks, observed)
                    result["slot"] = slot
                    rows.append(result)
                    descriptive = observation_findings(result["issues"]) if stage == "smoke" else []
                    summary["observation_issues"].extend(descriptive)
                    summary["issues"].extend(problem for problem in result["issues"] if problem not in descriptive)
                    summary["issues"].extend(invocation_limits(result, envelopes))
                    if stage != "smoke":
                        summary["issues"].extend(result["unavailable"])
                    sink_bytes = 0
                    if plan["loader"]:
                        sink_files = list(sinks.iterdir())
                        if len(sink_files) > 2:
                            summary["issues"].append(issue("storage", "more than two child sinks"))
                        for path in sink_files:
                            size = path.lstat().st_size
                            sink_bytes += size
                            if not path.is_file() or path.is_symlink() or size > plan["bounds"]["child_sink_bytes"]:
                                summary["issues"].append(issue("storage", "child sink exceeds reserved regular-file bound"))
                    validation_path = output / (slot["id"] + ".validation.json")
                    valid_ref = save(validation_path, result, 256 * 1024)
                    attempt["validation"] = {"name": validation_path.name, "sha256": valid_ref["sha256"]}
                    attempt["state"] = ("failed" if any(problem not in descriptive for problem in result["issues"])
                                        else "semantic_contract_passed")
                    summary["evidence_bytes"] += raw_ref["bytes"] + valid_ref["bytes"] + sink_bytes
                    summary["issues"].extend(verify_inputs(
                        plan, retention_deadline))
                    if time.monotonic() > retention_deadline:
                        summary["issues"].append(issue("deadline", "attempt retention reservation exceeded"))
                    if summary["evidence_bytes"] > reservation - 4 * MIB:
                        summary["issues"].append(issue("storage", "phase evidence reservation exceeded"))
                    if summary["issues"]:
                        break
                checkpoint = {"block": offset // 2, "attempted": len(summary["attempts"]),
                              "unrun": len(summary["unrun"]), "issues": summary["issues"].copy(),
                              "elapsed_seconds": time.monotonic() - started, "qualification": False}
                checkpoint_ref = save(output / (f"block-{offset // 2:02d}.json"), checkpoint, 256 * 1024)
                summary["evidence_bytes"] += checkpoint_ref["bytes"]
                yield checkpoint
                if summary["issues"]:
                    break
            if not summary["unrun"] and stage == "calibration" and not summary["issues"]:
                try:
                    summary["envelopes"] = derive_envelopes(rows, review, plan["case"])
                except (KeyError, TypeError, ValueError) as error:
                    # Insufficient repeatability stays inconclusive; only other refusals are numerical.
                    summary["issues"].append(issue(getattr(error, "category", "numerical"), str(error)))
            if stage in ("control", "comparison"):
                summary["issues"].extend(grade_rows(rows, stage, envelopes, plan["case"]))
        except BaseException as error:
            summary["issues"].append(issue("interrupted" if isinstance(error, (KeyboardInterrupt, GeneratorExit)) else "collector", str(error) or type(error).__name__))
            if summary["attempts"] and summary["attempts"][-1]["raw"] is None:
                summary["issues"].append(issue("cleanup", "launch outcome unknown; started receipt retained"))
        finally:
            final_deadline = min(deadline, time.monotonic() + 60)
            if active_runtime is not None:
                bundle, role_name, observed = active_runtime
                if not observed.get("post_checked"):
                    post = capture_runtime(plan, bundle, role_name, final_deadline)
                    observed["post"] = post["value"]
                    observed["post_checked"] = True
                    observed["issues"].extend(post["issues"])
                summary["issues"].extend(observed["issues"])
                summary["final_runtime"] = save(output / "interrupted-runtime.json", observed, 2 * MIB)
            summary["issues"].extend(verify_inputs(plan, final_deadline))
            if summary["unrun"]:
                summary["issues"].append(issue("incomplete", "original schedule has unrun slots"))
            if time.monotonic() > deadline:
                summary["issues"].append(issue("deadline", "original whole-phase deadline exceeded"))
            for failure in summary["issues"]:
                if failure["category"] in ("identity", "cleanup"):
                    campaign.fence(failure["category"], failure["reason"], phase)
            if stage == "control" and summary["issues"] and registration is not None:
                campaign.fence("control", "failed/incomplete unchanged control blocks all candidate cases", phase)
            summary["elapsed_seconds"] = time.monotonic() - started
            summary["statistics"] = cohort_statistics(rows, plan["case"], central_four=review is not None)
            summary["qualification"] = stage == "comparison" and not summary["issues"] and not summary["admission_issues"]
            summary.update(phase_outcomes(summary))
            seal = save(output / "summary.json", summary)
            if registration is not None and registration.exists():
                save(registration.with_suffix(".complete"), seal, 64 * 1024)
    return summary


def collect(plan, stage, output):
    with contextlib.closing(collect_steps(plan, stage, output)) as steps:
        while True:
            try:
                checkpoint = next(steps)
                print(document(checkpoint).decode(), end="", flush=True)
            except StopIteration as finished:
                return finished.value


def profile_proposal(calibration, control, *, check_fences=True):
    require(calibration["summary"]["stage"] == "calibration" and control["summary"]["stage"] == "control", "calibration/control stages required")
    require(calibration["plan"]["collector"] == collector_identity(), "collector source applicability changed")
    compatible(calibration, control)
    require(not calibration["issues"] and not control["issues"], "failed/unavailable cohort cannot be frozen")
    require(control["plan"].get("calibration") == {
        "directory": calibration["directory"], "binding": calibration["binding"]}, "control calibration binding mismatch")
    envelopes = derive_envelopes(calibration["rows"], calibration["review"], calibration["plan"]["case"])
    require(not grade_rows(control["rows"], "control", envelopes, calibration["plan"]["case"]), "unchanged control failed")
    campaign = Campaign(calibration["plan"])
    require(not check_fences or not campaign.fences(), "campaign identity/cleanup/control fence")
    return {"format": "consumer-profile-v1", "context_sha256": context_digest(calibration["plan"]),
            "case": calibration["plan"]["case"], "admission": calibration["plan"]["admission"],
            "calibration": {"directory": calibration["directory"], "binding": calibration["binding"]},
            "control": {"directory": control["directory"], "binding": control["binding"]}, "envelopes": envelopes}


def freeze(calibration_dir, control_dir, review_path, output):
    proposal = profile_proposal(load_cohort(calibration_dir), load_cohort(control_dir))
    review = load(review_path)
    require(review.get("accepted") is True and review.get("profile_proposal_sha256") == hashed(proposal),
            "external acceptance must bind exact numerical proposal")
    nonempty(review.get("reviewer"), "reviewer")
    nonempty(review.get("rationale"), "rationale")
    return save(output, {**proposal, "proposal_sha256": hashed(proposal), "acceptance": reference(review_path)})


def discover_runtime(plan, role_name, output):
    """One finite, nonnumerical actual invocation; no retry or implicit successor."""
    started = time.monotonic()
    deadline = started + 225
    validate_plan(plan)
    require(plan["loader"] and plan["target"] == runtime.APPLE, "Apple loader-on discovery required")
    require(plan.get("runtime_closure") is None, "discovery cannot replace a frozen runtime closure")
    require(role_name in plan["roles"], "explicit discovery role required")
    directory = Path(output).resolve()
    directory.mkdir(parents=False, exist_ok=False)
    plan_ref = save(directory / "plan.json", plan)
    sink_root = directory / "sinks"
    sink_root.mkdir()
    campaign = Campaign(plan)
    result = {"kind": "consumer-runtime-discovery-failure-v1", "qualification": False, "issues": [],
              "role": role_name, "execution_binding": runtime.execution_binding(plan),
              "raw": None, "sink_root": str(sink_root)}
    record, dispatched = None, False
    phase = plan["case"] + "-discovery-" + role_name
    registration = None
    with campaign.locked() as lock_issues:
        try:
            result["issues"].extend(lock_issues)
            if not lock_issues:
                result["issues"].extend(campaign.fences(diagnostic=True))
            result["issues"].extend(verify_inputs(plan, min(deadline, started + 60)))
            require(not result["issues"], "discovery input/campaign admission failed")
            path = campaign.root / "phases" / (phase + ".json")
            save(path, {"output": str(directory), "stage": "discovery", "summary_name": "discovery.json",
                        "plan_sha256": plan_ref["sha256"]}, 64 * 1024)
            registration = path
            result["registration"] = reference(path)
            require(shutil.disk_usage(directory).free >= (48 + 4 + 64) * MIB, "discovery storage reservation unavailable")
            runtime.check_node_resolution(plan, role_name)
            before = runtime.system_snapshot()
            require(time.monotonic() <= started + 60, "discovery initial admission deadline")
            role = plan["roles"][role_name]
            env = {**plan["env"], "MADO_P050_STDERR_SINK_DIR": str(sink_root)}
            result["started"] = save(directory / "started.json", {
                "role": role_name, "execution_binding": runtime.execution_binding(plan),
                "argv": role["argv"], "env": env, "system_before": before,
                "launch_disposition": "unknown"}, 64 * 1024)
            now = time.monotonic()
            if now > started + plan["bounds"]["admission_seconds"] or not admission_room(now, deadline, 1, plan["bounds"]):
                result["launch_disposition"] = "not_launched"
                raise ContractError("deadline", "discovery full launch/cleanup/retention reservation unavailable")
            dispatched = True
            record = run_process(role["argv"], cwd=Path(plan["root"]), env=env,
                                 timeout_seconds=plan["bounds"]["primary_seconds"],
                                 cleanup_seconds=plan["bounds"]["cleanup_seconds"],
                                 output_limit_bytes=plan["bounds"]["output_limit_bytes"])
            if record.get("cleanup_ok") is not True:
                result["issues"].append(issue("cleanup", "discovery outer cleanup unconfirmed"))
            result["raw"] = save(directory / "raw.json", {"role": role_name, "execution_binding": runtime.execution_binding(plan),
                "execution_limits": {"timeout_seconds": plan["bounds"]["primary_seconds"],
                                     "cleanup_seconds": plan["bounds"]["cleanup_seconds"],
                                     "output_limit_bytes": plan["bounds"]["output_limit_bytes"]},
                "record": record}, 25 * MIB)
            parsed = strict_json(record["stdout"])
            result["issues"].extend(terminal_issues(plan, role, parsed))
            parsed = semantic_output(plan, role_name, record)
            data = runtime.discover(plan, role_name, record, parsed, sink_root, before, min(deadline, time.monotonic() + 60))
            result.update(data)
        except BaseException as error:
            result["issues"].append(issue(getattr(error, "category", "discovery"), str(error) or type(error).__name__))
            if dispatched and record is None:
                result["issues"].append(issue("cleanup", "discovery launch outcome unknown"))
        finally:
            result["issues"].extend(verify_inputs(plan, min(deadline, time.monotonic() + 60)))
            if time.monotonic() > deadline:
                result["issues"].append(issue("deadline", "discovery whole deadline exceeded"))
            for problem in result["issues"]:
                if problem["category"] in ("identity", "cleanup"):
                    campaign.fence(problem["category"], problem["reason"], phase)
            result["elapsed_seconds"] = time.monotonic() - started
            seal = save(directory / "discovery.json", result)
            if registration is not None:
                save(registration.with_suffix(".complete"), seal, 64 * 1024)
    return result


def close_runtime(plan, paths, output):
    """Freeze explicit actual discoveries as data, never as method acceptance."""
    deadline = time.monotonic() + 60
    validate_plan(plan)
    require(not verify_inputs(plan, deadline), "closure source/collector identity changed")
    discoveries, refs = [], []
    require(0 < len(paths) <= 12, "finite discovery inputs required")
    for path in paths:
        ref = reference(path)
        require(ref not in refs, "duplicate discovery receipt")
        discovery = read_reference(ref)
        require(discovery.get("issues") == [], "failed/incomplete discovery cannot define a closure")
        raw = read_reference(discovery["raw"], 25 * MIB)
        runtime.validate_discovery_raw(plan, discovery, raw)
        parsed = semantic_output(plan, discovery["role"], raw["record"])
        discoveries.append(runtime.validate_discovery(plan, discovery, raw["record"], parsed, discovery["sink_root"]))
        refs.append(ref)
    bundle = runtime.closure_from(plan, discoveries)
    bundle["origins"] = refs
    bundle["freeze"] = {role: runtime.snapshot(plan, bundle, role, deadline) for role in plan["roles"]}
    require(not verify_inputs(plan, deadline), "closure final source identity check failed")
    return save(output, bundle)


def prepare(source_plan, case, campaign, output, *, loader=False, node=None):
    """Import commands and guarded inputs, not old developmental observations."""
    source_path = Path(source_plan).resolve(strict=True)
    source = load(source_path, 16 * MIB)
    require(case in CASES, "fixed actual-consumer case required")
    roles = {}
    for slot in source["runs"]:
        if not slot.get("observed"):
            continue
        argv = slot["argv"]
        if slot["kind"] == "trial":
            selected = "trial-" + argv[argv.index("--case") + 1]
        else:
            scenario = load(argv[2])
            selected = scenario["profile"] if slot["kind"] in ("matched", "deadline") else slot["kind"] + "-" + scenario["profile"]
        if selected != case or slot["role"] in roles:
            continue
        binary = source["binaries"][slot["role"] + "-consumer"]
        root_binary = source["binaries"][slot["role"] + ("-trial" if slot["kind"] == "trial" else "-consumer")]
        roles[slot["role"]] = {"argv": argv, "engine": binary, "root_binary": root_binary}
    require("baseline" in roles, "source plan has no observation-on baseline for selected case")
    corpus = None
    if case.startswith("trial-"):
        fixture = load(roles["baseline"]["argv"][4])
        corpus = fixture["corpus_sha256"]
    else:
        scenario = load(roles["baseline"]["argv"][2])
        corpus_id = scenario["native_config"]["replay"]["corpus_id"]
        require(corpus_id.startswith("p050-session-rgba-sha256:"), "source-bound replay corpus required")
        corpus = corpus_id.removeprefix("p050-session-rgba-sha256:")
    env = dict(source["env"])
    env["MADO_P050_OBSERVATIONS"] = "1"
    env.pop("MADO_P050_STDERR_SINK_DIR", None)
    env.pop("DYLD_PRINT_LIBRARIES", None)
    if loader:
        env["DYLD_PRINT_LIBRARIES"] = "1"
    inputs = dict(source["inputs"])
    inputs[str(source_path)] = {"sha256": file_digest(source_path)}
    for role in roles.values():
        for name in ("engine", "root_binary"):
            binary = role[name]
            inputs[binary["path"]] = {"sha256": binary["sha256"]}
        if node is not None:
            path = str(Path(node).resolve(strict=True))
            role["node"] = {"path": path, "sha256": file_digest(path)}
            inputs[path] = {"sha256": role["node"]["sha256"]}
    plan = {"format": "actual-consumer-plan-v1", "execution_authorized": source.get("execution_authorized") is True,
            "case": case, "target": host_identity()["target"], "host": host_identity(), "configuration": CONFIGURATION,
            "root": source["root"], "env": env, "inputs": inputs, "roles": roles,
            "collector": collector_identity(), "bounds": LIMITS.copy(), "fixture": {"corpus_sha256": corpus},
            "loader": loader, "campaign": str(Path(campaign).resolve()), "admission": None, "runtime_closure": None,
            "source_plan": reference(source_path), "calibration": None, "profile": None}
    validate_plan(plan)
    if loader:
        require(plan["target"] == runtime.APPLE, "actual consumer runtime closure supports Apple only")
        for role in roles:
            runtime.check_node_resolution(plan, role)
    return save(output, plan)


def inspect_retained(plan_path, raw_path, sink_root=None):
    plan = load(plan_path)
    validate_plan(plan)
    raw = load(raw_path, 25 * MIB)
    require(isinstance(raw.get("slot"), dict) and isinstance(raw.get("record"), dict),
            "retained source-bound slot/record envelope required")
    role = raw["slot"]["role"]
    require(role in plan["roles"], "retained role absent from explicit plan")
    slot = {"id": "retained", "role": role, "process": 0, "pair": 0, "position": 0}
    review, missing = admission(plan)
    result = reconcile(plan, slot, raw["record"], review, Path(sink_root).resolve() if sink_root else None)
    return {"kind": "retained-development-inspection", "qualification": False,
            "plan": reference(plan_path), "raw": reference(raw_path), "original_slot": raw["slot"],
            "issues": result["issues"], "admission_issues": missing, "observation": result,
            "note": "Retained developmental observations are not calibration or new execution."}


def parser():
    result = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = result.add_subparsers(dest="command", required=True)
    draft = commands.add_parser("prepare", help="create a new development-only plan from explicit retained command/identities")
    draft.add_argument("--source-plan", required=True)
    draft.add_argument("--case", required=True, choices=CASES)
    draft.add_argument("--campaign", required=True)
    draft.add_argument("--output", required=True)
    draft.add_argument("--loader", action="store_true", help="enable actual Apple dyld logging, never qualification")
    draft.add_argument("--node", help="explicit executable actually selected by PATH for replay's Node child")
    discovery = commands.add_parser("discover", help="one actual loader-on invocation, data only")
    discovery.add_argument("--plan", required=True)
    discovery.add_argument("--role", required=True, choices=("baseline", "candidate"))
    discovery.add_argument("--output", required=True)
    closure = commands.add_parser("close-runtime", help="freeze explicit actual discovery images without accepting a method")
    closure.add_argument("--plan", required=True)
    closure.add_argument("--discovery", action="append", required=True)
    closure.add_argument("--output", required=True)
    for command in ("smoke", "calibrate", "control", "compare"):
        sub = commands.add_parser(command)
        sub.add_argument("--plan", required=True)
        sub.add_argument("--output", required=True)
    frozen = commands.add_parser("freeze")
    frozen.add_argument("--calibration", required=True)
    frozen.add_argument("--control", required=True)
    frozen.add_argument("--review", required=True)
    frozen.add_argument("--output", required=True)
    proposed = commands.add_parser("profile-proposal", help="print exact proposal for independent acceptance; no launch")
    proposed.add_argument("--calibration", required=True)
    proposed.add_argument("--control", required=True)
    report = commands.add_parser("report", help="reconcile retained raw schema without launching or changing receipts")
    report.add_argument("directory")
    inspect = commands.add_parser("inspect", help="read an original retained slot/record without promoting it to calibration")
    inspect.add_argument("--plan", required=True)
    inspect.add_argument("--raw", required=True)
    inspect.add_argument("--sink-root")
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        if args.command == "prepare":
            value = prepare(args.source_plan, args.case, args.campaign, args.output, loader=args.loader, node=args.node)
        elif args.command == "discover":
            value = discover_runtime(load(args.plan), args.role, args.output)
        elif args.command == "close-runtime":
            value = close_runtime(load(args.plan), args.discovery, args.output)
        elif args.command == "inspect":
            value = inspect_retained(args.plan, args.raw, args.sink_root)
        elif args.command == "freeze":
            value = freeze(args.calibration, args.control, args.review, args.output)
        elif args.command == "profile-proposal":
            value = profile_proposal(load_cohort(args.calibration), load_cohort(args.control))
            value = {"proposal": value, "profile_proposal_sha256": hashed(value)}
        elif args.command == "report":
            evidence = load_cohort(args.directory)
            value = {"stage": evidence["summary"]["stage"], "case": evidence["plan"]["case"],
                     "issues": evidence["issues"], "admission_issues": evidence["summary"]["admission_issues"],
                     "rows": evidence["rows"], "unrun": evidence["summary"]["unrun"],
                     "statistics": cohort_statistics(evidence["rows"], evidence["plan"]["case"],
                                                     central_four=evidence["review"] is not None),
                     "qualification": evidence["summary"]["qualification"] and not evidence["issues"]}
            if evidence["summary"]["format"] == "consumer-phase-v2":
                value.update(format="consumer-phase-v2",
                             observation_issues=evidence["summary"]["observation_issues"],
                             **phase_outcomes({**evidence["summary"], "issues": evidence["issues"]}))
        else:
            stage = {"calibrate": "calibration", "compare": "comparison"}.get(args.command, args.command)
            value = collect(load(args.plan), stage, args.output)
        print(document(value).decode(), end="")
        return int(bool(value.get("issues")))
    except (KeyError, IndexError, TypeError, ValueError, OSError, runtime.images._Refusal) as error:
        print(document({"qualification": False, "error": str(error)}).decode(), end="", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
