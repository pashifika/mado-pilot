"""Evaluate retained Rust OCR observations as independent per-rule outcomes.

Original JSONL, process receipts and Git objects are the authority; supplied
verdicts and summaries are not. Outcomes describe test evidence only and never
decide model availability. Report schema 1 is the historical admission-era
runner output: its identity ``accepted`` field is a historical fact, not policy.
"""
from __future__ import annotations

import base64
import hashlib
import json
import math
import os
import subprocess
import unicodedata
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]
CRITERIA = json.loads((Path(__file__).with_name("criteria-v1.json")).read_text(encoding="utf-8"))
BUILD_SOURCE_FIELDS = ("build_source_revision", "build_source_tree", "build_compiler", "build_source_dirty")
BUILD_FIELDS = ("catalog_sha256", "executable_sha256", "opencv") + BUILD_SOURCE_FIELDS
IDENTITY_FIELDS = ("source_revision", "source_tree", "executable_sha256", "runtime_sha256",
                   "catalog_sha256", "criteria_sha256", "fixtures_sha256", "compiler", "opencv",
                   "build_source_revision", "build_source_tree", "build_compiler")
COHORT_FIELDS = ("cohort_id", "target", "os", "cpu", "host_id", "runtime", "provider", "intra_op", "inter_op")
PROCESS_METRICS = ("cold_open_ms", "close_ms", "peak_resident_bytes", "model_bytes", "auxiliary_bytes")
BUDGET_PROCESS_METRICS = ("cold_open_ms", "close_ms", "peak_resident_bytes")
BUDGET_WORKLOAD_METRICS = ("p50_ms", "p95_ms", "max_ms", "rust_peak_allocated_bytes", "rust_growth_bytes", "rust_steady_bytes", "mapped_bytes", "detector_tensor_bytes")
POSITIVE_SAMPLE_METRICS = ("detector_width", "detector_height", "detector_tensor_bytes", "detector_runs", "mapped_bytes")
EVALUATION_SCHEMA_VERSION = 1
# Report schema version -> execution value of a complete, non-diagnostic campaign.
COMPLETE_EXECUTION = {1: "qualification", 2: "evaluation"}
DESCRIPTOR_FIELDS = frozenset(("profile_id", "model_id", "model_version", "preprocessing_id", "decoder_id",
                               "normalization_id", "execution_sha256", "resources", "limits"))
COLLECTOR_SCOPE = {"rust_accounting": "SDK plus raw observation construction/serialization",
                   "native_peak_rss": "entire fresh process including cold initialization, workloads and lifecycle"}
# Quality and numerical failures are findings of a successful evaluation;
# a failed technical contract is also an evaluation error.
RULES = (("text_order", "quality"), ("region_count", "quality"), ("geometry", "quality"),
         ("unexpected_detection", "quality"), ("confidence", "quality"), ("stability", "quality"),
         ("vocabulary_coverage", "quality"), ("source_correlation", "technical"), ("lifecycle", "technical"),
         ("execution_bounds", "technical"), ("process_budgets", "numerical"), ("workload_budgets", "numerical"),
         ("relative_latency", "numerical"))
RULE_IDS = tuple(rule for rule, _ in RULES)
OBSERVATION_RULES = ("text_order", "region_count", "geometry", "unexpected_detection", "confidence",
                     "stability", "source_correlation")
SAMPLE_METRIC_RULES = ("execution_bounds", "workload_budgets", "relative_latency")
SAMPLE_RULES = OBSERVATION_RULES + SAMPLE_METRIC_RULES


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def evaluator_revision():
    """Content address of the evaluation semantics, independent of Git state."""
    return "sha256:" + sha256(Path(__file__))


def document_bytes(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n").encode("utf-8")


def frozen_corpus():
    root = Path(__file__).resolve().parents[2]
    result = []
    for directory in ("g-004", "compact-ui-v1"):
        manifest = json.loads((root / f"fixtures/ocr/{directory}/fixture-manifest.json").read_text(encoding="utf-8"))
        for image in manifest["images"]:
            result.append({"id": f"{manifest['fixture_profile_id']}/{Path(image['file']).stem}",
                           "sha256": image["sha256"], "width": image["width"],
                           "byte_len": (root / f"fixtures/ocr/{directory}" / image["file"]).stat().st_size,
                           "height": image["height"], "regions": image["regions"]})
    return result


def corpus_identities():
    root = Path(__file__).resolve().parents[2]
    result = []
    for directory in ("g-004", "compact-ui-v1"):
        path = root / f"fixtures/ocr/{directory}/fixture-manifest.json"
        result.append({"profile": json.loads(path.read_text(encoding="utf-8"))["fixture_profile_id"], "sha256": sha256(path)})
    return result


def strict_json(text):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key {key}")
            result[key] = value
        return result

    def finite_float(value):
        parsed = float(value)
        if not math.isfinite(parsed):
            raise ValueError(f"nonfinite JSON {value}")
        return parsed

    try:
        return json.loads(text, object_pairs_hook=unique, parse_float=finite_float,
                          parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"nonfinite JSON {value}")))
    except (ArithmeticError, RecursionError) as error:
        raise ValueError(f"invalid JSON: {type(error).__name__}") from error


def _describe(error):
    """Error text without host-local paths."""
    if isinstance(error, OSError):
        return error.strerror or type(error).__name__
    return str(error)


def _mapping(value):
    return value if isinstance(value, dict) else {}


def _json_safe(value):
    return repr(value) if isinstance(value, float) and not math.isfinite(value) else value


def parse_process(raw_path, operational, command, index, *, evidence_root):
    result = {"index": index, "process": operational, "command": command,
              "samples": [], "workloads": {}, "raw_records": [],
              "raw_path": raw_path.resolve().relative_to(Path(evidence_root).resolve()).as_posix()}
    try:
        raw = raw_path.read_bytes()
        result["raw_sha256"] = hashlib.sha256(raw).hexdigest()
        for line in raw.decode("utf-8").splitlines():
            record = strict_json(line)
            kind = record.get("type")
            if kind == "meta":
                if "pid" in result:
                    raise ValueError("duplicate Rust meta")
                keys = {"index", "pid", "binding", "profile", "identity", "build", "target", "opencv", "runtime", "provider"}
                if set(record) != keys | {"type"}:
                    raise ValueError("invalid Rust meta fields")
                result.update({key: record[key] for key in keys})
            elif kind == "collector-scope":
                if "collector_scope" in result:
                    raise ValueError("duplicate collector scope")
                result["collector_scope"] = {key: value for key, value in record.items() if key != "type"}
            elif kind == "sample":
                result["samples"].append(record)
            elif kind == "workload":
                if record["workload"] in result["workloads"]:
                    raise ValueError("duplicate workload metrics")
                result["workloads"][record["workload"]] = record["metrics"]
            elif kind == "vocabulary":
                if "vocabulary_supported" in result:
                    raise ValueError("duplicate vocabulary record")
                result["vocabulary_supported"] = record["supported"]
            elif kind == "terminal":
                if "lifecycle" in result:
                    raise ValueError("duplicate terminal record")
                for key in ("lifecycle", "lifecycle_detail", "metrics", "close_fault"):
                    result[key] = record[key]
            elif kind == "failure":
                result["raw_records"].append(record)
            else:
                raise ValueError(f"unknown Rust record {kind}")
    except (ValueError, TypeError, KeyError, AttributeError, OSError, UnicodeError) as error:
        result["parse_failure"] = _describe(error)
    return result


def _safe_relative(relative):
    return (isinstance(relative, str) and bool(PurePosixPath(relative).parts)
            and not PurePosixPath(relative).is_absolute()
            and not any(part in ("", ".", "..") for part in relative.split("/"))
            and "\\" not in relative and ":" not in relative)


def evidence_path(root, relative, *, missing_ok=False):
    """Contained regular evidence file; ``missing_ok`` returns the lexical path of an absent file."""
    if not _safe_relative(relative):
        raise ValueError("missing/unsafe relative evidence path")
    path = root / relative
    if missing_ok and not os.path.lexists(path):
        return path
    try:
        resolved = path.resolve(strict=True)
        resolved.relative_to(root)
    except OSError:
        raise ValueError(f"evidence file unavailable: {relative}") from None
    except ValueError:
        raise ValueError(f"evidence path escapes the evidence root: {relative}") from None
    if not resolved.is_file():
        raise ValueError("evidence path is not a regular file")
    return resolved


def reconcile_raw(report, evidence_root):
    """Verified original processes per profile plus (profile, process, message) integrity errors.

    A process whose original receipt or JSONL cannot be verified is excluded; the
    report's own projection is never evaluated.
    """
    if evidence_root is None:
        return {}, [(None, None, "original raw JSONL evidence root is required")]
    try:
        root = Path(evidence_root).resolve(strict=True)
    except OSError:
        return {}, [(None, None, "original evidence root unavailable")]
    errors, profiles, retained, receipts = [], {}, set(), set()
    for profile, processes in report.get("profiles", {}).items():
        verified = profiles.setdefault(profile, [])
        for process in processes:
            index = process.get("index") if isinstance(process, dict) else None
            stage = "raw JSONL"
            try:
                absent = "raw_sha256" not in process
                path = evidence_path(root, process.get("raw_path"), missing_ok=True)
                if path.suffix != ".jsonl" or path in retained:
                    raise ValueError("duplicate/non-JSONL raw evidence path")
                if path.is_file():
                    retained.add(path)
                stage = "process receipt"
                receipt_path = evidence_path(root, process.get("receipt_path"), missing_ok=True)
                if not receipt_path.name.endswith(".process.json") or receipt_path in receipts:
                    raise ValueError("duplicate/invalid process receipt path")
                if receipt_path.is_file():
                    receipts.add(receipt_path)
                stage = "raw JSONL"
                if absent and os.path.lexists(path):
                    raise ValueError("retained raw JSONL has no reported digest")
                if not absent:
                    path = evidence_path(root, process.get("raw_path"))
                stage = "process receipt"
                receipt_bytes = receipt_path.read_bytes()
                receipt_sha256 = hashlib.sha256(receipt_bytes).hexdigest()
                if receipt_sha256 != process.get("receipt_sha256"):
                    errors.append((profile, index, "original process receipt digest mismatch"))
                receipt = strict_json(receipt_bytes.decode("utf-8"))
                if not isinstance(receipt, dict) or set(receipt) != {"index", "process", "command"}:
                    raise ValueError("invalid process receipt fields")
                command = receipt["command"]
                invoked_profiles = [command[position + 1] if position + 1 < len(command) else None
                                    for position, argument in enumerate(command) if argument == "--profile"
                                    ] if isinstance(command, list) else []
                if invoked_profiles != [profile]:
                    errors.append((None, None, "original process invocation/profile attribution mismatch"))
                stage = "raw JSONL"
                original = parse_process(path, receipt["process"], receipt["command"],
                                         receipt["index"], evidence_root=root)
                if "profile" in original and original["profile"] != profile:
                    errors.append((None, None, "original Rust meta/profile attribution mismatch"))
                if original["index"] != receipt["index"]:
                    errors.append((profile, index, "raw observation/process receipt index mismatch"))
                original.update(receipt_path=receipt_path.relative_to(root).as_posix(),
                                receipt_sha256=receipt_sha256)
                if original.get("raw_sha256") != process.get("raw_sha256"):
                    errors.append((profile, index, "original raw JSONL digest mismatch"))
                # Absent-file failure text was host-specific when recorded.
                ignored = {"parse_failure"} if absent else set()
                differing = sorted(key for key in (set(original) | set(process)) - ignored
                                   if key not in original or key not in process or original[key] != process[key])
                if differing:
                    errors.append((profile, index, f"original raw JSONL projection mismatch: {', '.join(differing)}"))
                verified.append(original)
            except (ValueError, TypeError, KeyError, AttributeError, OSError, UnicodeError) as error:
                errors.append((profile, index, f"original {stage} unavailable/invalid: {_describe(error)}"))
    if {path.resolve() for path in root.rglob("*.jsonl")} != retained:
        errors.append((None, None, "original raw JSONL attempt set differs from retained report"))
    if {path.resolve() for path in root.rglob("*.process.json")} != receipts:
        errors.append((None, None, "original process receipt attempt set differs from retained report"))
    return profiles, errors


def git_bytes(repository_root, *arguments):
    try:
        result = subprocess.run(["git", "--no-replace-objects", "-C", str(repository_root), *arguments],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=False)
    except OSError as error:
        raise ValueError(f"Git authority unavailable: {_describe(error)}") from error
    except subprocess.TimeoutExpired as error:
        raise ValueError("Git authority timed out") from error
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip().replace(str(repository_root), "<repository>")
        raise ValueError(f"Git authority failed ({' '.join(arguments)}): {detail}")
    return result.stdout


def git_hex(value):
    return isinstance(value, str) and len(value) == 40 and all(c in "0123456789abcdef" for c in value)


def _sha256_hex(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def number(value, *, positive=False) -> bool:
    try:
        return (isinstance(value, (int, float)) and not isinstance(value, bool)
                and math.isfinite(value) and (value > 0 if positive else value >= 0))
    except OverflowError:
        return False


def percentile(values, fraction):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)]


def _area(polygon):
    return abs(sum(a[0] * b[1] - b[0] * a[1] for a, b in zip(polygon, polygon[1:] + polygon[:1]))) / 2


def _intersection(subject, clip):
    # Convex polygon clipping; fixture quads use clockwise image coordinates.
    for a, b in zip(clip, clip[1:] + clip[:1]):
        incoming, subject = subject, []
        if not incoming:
            break
        def side(p):
            return (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        previous = incoming[-1]
        for current in incoming:
            old, new = side(previous), side(current)
            if (old >= 0) != (new >= 0):
                ratio = old / (old - new)
                subject.append([previous[0] + ratio * (current[0] - previous[0]),
                                previous[1] + ratio * (current[1] - previous[1])])
            if new >= 0:
                subject.append(current)
            previous = current
    return subject


def _quality_text(text):
    return unicodedata.normalize("NFC", text).strip().replace("％", "%")


def _observation_gap(observation):
    """Why a sample holds no recognition result to compare, else None."""
    if observation.get("status") != "Ok":
        return f"typed recognition failure {observation.get('status')!r}"
    regions = observation.get("regions")
    if not isinstance(regions, list) or not all(isinstance(region, dict) for region in regions):
        return "missing/malformed raw regions"
    return None


def quality(observation, expected, mode):
    """Typed rule findings for one successful observation (see ``_observation_gap``)."""
    findings = []

    def fail(rule, kind, **facts):
        findings.append({"rule_id": rule, "kind": kind, **facts})

    if observation.get("source_correlation") is not True:
        fail("source_correlation", "result_correlation", observed=observation.get("source_correlation"))
    source = observation.get("source")
    source_fields = ("stamp", "transform", "effective_region", "backend", "output_space", "effective_zones")
    if (not isinstance(source, dict) or any(not source.get(key) for key in source_fields)
            or observation.get("result_source") != source):
        fail("source_correlation", "source_identity")
    actual = observation["regions"]
    if len(actual) != len(expected):
        fail("region_count", "count", expected=len(expected), observed=len(actual))
    for index, region in enumerate(actual):
        confidence = region.get("confidence")
        if not number(confidence) or confidence > 1:
            fail("confidence", "range", region=index, observed=_json_safe(confidence))
        quad = region.get("quad")
        valid_quad = (isinstance(quad, list) and len(quad) == 4 and all(
            isinstance(point, list) and len(point) == 2
            and all(number(v) and v <= 1 for v in point) for point in quad))
        if not valid_quad:
            fail("geometry", "invalid_quad", region=index)
        if index >= len(expected):
            if number(confidence) and confidence >= 0.5:
                fail("unexpected_detection", "unmatched", region=index, observed=confidence, limit=0.5)
            continue
        oracle = expected[index]
        text = region.get("text")
        if not isinstance(text, str) or _quality_text(text) != _quality_text(oracle["text_nfc"]):
            fail("text_order", "text", region=index, expected=oracle["text_nfc"], observed=text)
        if not valid_quad:
            continue
        target = oracle["source_relative_quad"]
        intersection = _area(_intersection(quad, target))
        union = _area(quad) + _area(target) - intersection
        if union <= 0 or intersection / union < 0.5:
            fail("geometry", "iou", region=index, observed=intersection / union if union > 0 else None, limit=0.5)
        for axis in (0, 1):
            center = (min(p[axis] for p in quad) + max(p[axis] for p in quad)) / 2
            target_center = (min(p[axis] for p in target) + max(p[axis] for p in target)) / 2
            if abs(center - target_center) > 0.025:
                fail("geometry", "center_delta", region=index, axis="xy"[axis],
                     observed=abs(center - target_center), limit=0.025)
    if mode == "grouped" and observation.get("groups") != [list(range(len(actual))), list(range(len(actual)))]:
        fail("text_order", "groups", expected=[list(range(len(actual)))] * 2, observed=observation.get("groups"))
    return findings


def validate_binding(binding):
    errors = []
    for key in IDENTITY_FIELDS + COHORT_FIELDS:
        value = binding.get(key)
        valid = type(value) is int and value == 1 if key in ("intra_op", "inter_op") else isinstance(value, str) and bool(value.strip())
        if not valid:
            errors.append(f"missing/invalid binding {key}")
    for key in ("source_revision", "source_tree", "build_source_revision", "build_source_tree"):
        if not git_hex(binding.get(key)):
            errors.append(f"invalid binding {key}")
    for built, source in (("build_source_revision", "source_revision"), ("build_source_tree", "source_tree"),
                          ("build_compiler", "compiler")):
        if binding.get(built) != binding.get(source):
            errors.append(f"compiled build provenance mismatch: {built}")
    if binding.get("build_source_dirty") is not False:
        errors.append("compiled build source is dirty or unavailable")
    if binding.get("build_compiler") == "unavailable":
        errors.append("compiled build compiler is unavailable")
    for key in ("executable_sha256", "runtime_sha256", "catalog_sha256", "criteria_sha256", "fixtures_sha256"):
        if not _sha256_hex(binding.get(key, "")):
            errors.append(f"invalid binding {key}")
    if binding.get("target") not in CRITERIA["release_targets"]:
        errors.append("wrong release target")
    if binding.get("provider") != "cpu" or binding.get("intra_op") != 1 or binding.get("inter_op") != 1:
        errors.append("CPU thread-one policy missing")
    if binding.get("qualified_host") is not True or not isinstance(binding.get("host_approval"), str) or not binding["host_approval"].strip():
        errors.append("host is not qualified/approved")
    if binding.get("serial_execution") is not True:
        errors.append("serial process execution policy missing")
    return errors


def validate_budgets(budgets, binding, workload_ids):
    errors = []
    if not isinstance(budgets, dict):
        return ["missing precommitted absolute budgets"]
    if not isinstance(budgets.get("approval"), str) or not budgets["approval"].strip() or not isinstance(budgets.get("frozen_revision"), str) or len(budgets["frozen_revision"]) != 40:
        errors.append("absolute budgets lack precommit/approval")
    for field in COHORT_FIELDS:
        if budgets.get("cohort", {}).get(field) != binding.get(field):
            errors.append(f"budget cohort mismatch: {field}")
    for key in BUDGET_PROCESS_METRICS:
        if not number(budgets.get("process", {}).get(key), positive=True):
            errors.append(f"missing process budget {key}")
    for workload in workload_ids:
        for key in BUDGET_WORKLOAD_METRICS:
            if not number(budgets.get("workloads", {}).get(workload, {}).get(key)):
                errors.append(f"missing workload budget {workload}/{key}")
    if set(budgets.get("workloads", {})) != set(workload_ids):
        errors.append("absolute budget workload set differs from frozen suite")
    if set(budgets.get("required_collectors", [])) != {"rust-accounting", "native-peak-rss", "onnx-stage-clocks"}:
        errors.append("required collector policy missing")
    return errors


class ProfileEvidence:
    """One profile's typed rule findings, evidence gaps and attribution errors."""

    def __init__(self):
        self.findings = {rule: [] for rule in RULE_IDS}
        self.gaps = []
        self.invalid = []
        self.processes = []
        self.metrics = {"processes": [], "workloads": {}}

    def fail(self, rule, kind, **facts):
        self.findings[rule].append({"kind": kind, **{key: _json_safe(value) for key, value in facts.items()}})

    def gap(self, rules, message, **location):
        """Missing evidence: ``rules`` cannot pass but keep their reliable findings."""
        self.gaps.append((tuple(rules), {**location, "message": message}))

    def unverified(self, message, **location):
        """Unattributable evidence: no rule of this profile can pass or fail."""
        self.invalid.append({**location, "message": message})

    def gap_rules(self):
        return {rule for rules, _ in self.gaps for rule in rules}


def _check_terminal(evidence, process, index, budgets, resources, lifecycle_bound):
    metrics = _mapping(process.get("metrics"))
    for key in PROCESS_METRICS:
        value = metrics.get(key)
        if not number(value, positive=key != "auxiliary_bytes"):
            if key in BUDGET_PROCESS_METRICS:
                evidence.gap(("process_budgets",), f"missing process metric {key}", process=index)
            continue
        limit = _mapping(budgets.get("process")).get(key)
        if key in BUDGET_PROCESS_METRICS and number(limit) and value > limit:
            evidence.fail("process_budgets", "ceiling", process=index, metric=key, observed=value, limit=limit)
    evidence.metrics["processes"].append({"index": index, **{key: metrics[key] for key in PROCESS_METRICS
                                                              if number(metrics.get(key))}})
    model_bytes = sum(resource.get("bytes", 0) for resource in resources if resource.get("kind") in ("detector", "recognizer"))
    auxiliary_bytes = sum(resource.get("bytes", 0) for resource in resources if resource.get("kind") == "auxiliary")
    if metrics.get("model_bytes") != model_bytes or metrics.get("auxiliary_bytes") != auxiliary_bytes:
        evidence.unverified("resource byte metrics differ from execution identity", process=index)
    lifecycle = process.get("lifecycle")
    if not isinstance(lifecycle, dict):
        evidence.gap(("lifecycle",), "malformed terminal lifecycle record", process=index)
    else:
        for operation, expected in dict(CRITERIA["required_lifecycle"], initialization_cancelled="Cancelled").items():
            if lifecycle.get(operation) != expected:
                evidence.fail("lifecycle", "status", process=index, operation=operation,
                              expected=expected, observed=lifecycle.get(operation))
    if process.get("close_fault") is not None:
        evidence.fail("lifecycle", "close_fault", process=index, observed=process["close_fault"])
    details = _mapping(process.get("lifecycle_detail"))
    for operation in ("cancelled", "deadline"):
        detail = details.get(operation)
        if not isinstance(detail, dict):
            evidence.gap(("lifecycle",), f"missing {operation} settlement collector", process=index)
            continue
        if detail.get("scope") != "admitted-before-native-entry" or detail.get("bound_ms") != lifecycle_bound:
            evidence.unverified(f"{operation} settlement collector scope/bound differs from frozen policy", process=index)
        for field in ("admitted", "termination_issued", "worker_joined"):
            value = detail.get(field)
            if value is False:
                evidence.fail("lifecycle", "settlement", process=index, operation=operation, field=field,
                              expected=True, observed=False)
            elif value is not True:
                evidence.gap(("lifecycle",), f"missing {operation} {field} observation", process=index)
        completions = detail.get("cleanup_completions")
        if type(completions) is not int:
            evidence.gap(("lifecycle",), f"missing {operation} cleanup observation", process=index)
        elif completions != 1:
            evidence.fail("lifecycle", "settlement", process=index, operation=operation,
                          field="cleanup_completions", expected=1, observed=completions)
        returned = detail.get("return_ms")
        if not number(returned):
            evidence.gap(("lifecycle",), f"missing {operation} return time", process=index)
        elif number(lifecycle_bound, positive=True) and returned > lifecycle_bound:
            evidence.fail("lifecycle", "settlement", process=index, operation=operation, field="return_ms",
                          observed=returned, limit=lifecycle_bound)


def _check_sample(evidence, row, at, spec, mode, limits, workload_limits):
    """Quality, execution-contract and per-sample ceiling findings for one retained row."""
    missing = _observation_gap(row)
    if missing:
        evidence.gap(SAMPLE_RULES, missing, **at)
    else:
        for finding in quality(row, spec["regions"], mode):
            rule = finding.pop("rule_id")
            evidence.fail(rule, finding.pop("kind"), **at, **finding)
    metrics = _mapping(row.get("metrics"))
    runs = metrics.get("detector_runs")
    if number(runs) and (runs > 1 or (not missing and runs != 1)):
        evidence.fail("execution_bounds", "detector_runs", **at, expected=1, observed=runs)
    mapped, expected_mapped = metrics.get("mapped_bytes"), spec["width"] * spec["height"] * 4
    if number(mapped) and (mapped > expected_mapped or (not missing and mapped != expected_mapped)):
        evidence.fail("execution_bounds", "mapped_bytes", **at, expected=expected_mapped, observed=mapped)
    width, height, tensor = metrics.get("detector_width"), metrics.get("detector_height"), metrics.get("detector_tensor_bytes")
    if number(width, positive=True) and number(height, positive=True):
        expected_tensor = width * height * 3 * 4
        if number(tensor) and (tensor > expected_tensor or (not missing and tensor != expected_tensor)):
            evidence.fail("execution_bounds", "detector_tensor_bytes", **at, expected=expected_tensor, observed=tensor)
    for axis, value in (("width", width), ("height", height)):
        limit = limits.get(f"detector_{axis}")
        if number(value) and number(limit, positive=True) and value > limit:
            evidence.fail("execution_bounds", f"detector_{axis}", **at, observed=value, limit=limit)
    tensor_limit = limits.get("tensor_bytes")
    if number(tensor_limit, positive=True) and number(tensor) and tensor > tensor_limit:
        evidence.fail("execution_bounds", "tensor_bytes", **at, observed=tensor, limit=tensor_limit)
    if not missing:
        recognizer_runs, expected_runs = metrics.get("recognizer_runs"), math.ceil(len(row["regions"]) / 6)
        if number(recognizer_runs) and recognizer_runs != expected_runs:
            evidence.fail("execution_bounds", "recognizer_runs", **at, expected=expected_runs, observed=recognizer_runs)
    for key in ("mapped_bytes", "detector_tensor_bytes"):
        value, limit = metrics.get(key), workload_limits.get(key)
        if number(value) and number(limit) and value > limit:
            evidence.fail("workload_budgets", "ceiling", **at, metric=key, observed=value, limit=limit)
    return missing is None


def evaluate_profile(processes, profile_id, binding, fixtures, budgets, identity, lifecycle_bound):
    """Observe one profile's verified original processes; missing data never becomes a pass."""
    evidence = ProfileEvidence()
    specs = {f"{row['id']}/{mode}": (row, mode) for row in fixtures for mode in ("singular", "grouped")}
    if len(processes) < CRITERIA["minimum_processes"]:
        evidence.gap(RULE_IDS, f"{len(processes)} of {CRITERIA['minimum_processes']} fresh processes retained")
    budgets = _mapping(budgets)
    workload_budgets = _mapping(budgets.get("workloads"))
    limits = _mapping(_mapping(identity).get("limits"))
    resources = _mapping(identity).get("resources")
    resources = [resource for resource in resources if isinstance(resource, dict)] if isinstance(resources, list) else []
    schedule = [(phase, i) for phase, count in (("warmup", CRITERIA["warmups"]), ("sample", CRITERIA["samples"]))
                for i in range(count)]
    stages = [key for key in CRITERIA["required_sample_metrics"] if key.endswith("_ms") and key != "elapsed_ms"]
    times = {name: [] for name in specs}
    stage_samples = {name: {stage: [] for stage in stages} for name in specs}
    observed_max = {name: {} for name in specs}
    indexes, pids, stable = set(), set(), {}

    def peak(name, key, value):
        observed_max[name][key] = max(value, observed_max[name].get(key, value))

    for process in processes:
        index = process.get("index")
        evidence.processes.append({"index": index, "raw_sha256": process.get("raw_sha256"),
                                   "receipt_sha256": process.get("receipt_sha256")})
        operational = _mapping(process.get("process"))
        if (operational.get("exit_code") != 0 or operational.get("timed_out") is not False
                or operational.get("output_limited") is not False or operational.get("cleanup_ok") is not True
                or operational.get("launch_error") is not None):
            evidence.gap(RULE_IDS, "bounded child failed (exit_code={!r}, timed_out={!r}, output_limited={!r}, "
                         "cleanup_ok={!r}, launch_failed={!r})".format(
                             operational.get("exit_code"), operational.get("timed_out"),
                             operational.get("output_limited"), operational.get("cleanup_ok"),
                             operational.get("launch_error") is not None), process=index)
        if process.get("parse_failure"):
            evidence.gap(RULE_IDS, f"original raw JSONL unavailable/unparsable: {process['parse_failure']}", process=index)
        for record in process.get("raw_records", []):
            evidence.gap(RULE_IDS, f"preserved Rust failure record (stage={record.get('stage')!r}, "
                         f"status={record.get('status')!r})", process=index)
        if not isinstance(process.get("command"), list) or not process["command"]:
            evidence.unverified("missing exact reproduction command", process=index)
        pid = process.get("pid")
        if index in indexes or ("pid" in process and (not isinstance(pid, int) or pid in pids)):
            evidence.unverified("duplicate/missing fresh process identity", process=index)
        indexes.add(index)
        pids.add(pid)
        if "pid" not in process:
            evidence.gap(RULE_IDS, "missing Rust meta record", process=index)
            continue
        if process.get("binding") != binding or process.get("profile") != profile_id:
            evidence.unverified("identity/cohort mismatch", process=index)
        if process.get("identity") != identity:
            evidence.unverified("execution/resource identity mismatch", process=index)
        if any(_mapping(process.get("build")).get(key) != binding.get(key) for key in BUILD_FIELDS):
            evidence.unverified("actual compiled build facts mismatch", process=index)
        if process.get("target") != binding.get("target") or process.get("opencv") != binding.get("opencv"):
            evidence.unverified("actual target/OpenCV mismatch", process=index)
        if process.get("runtime") != binding.get("runtime") or process.get("provider") != "cpu":
            evidence.unverified("actual runtime/provider mismatch", process=index)
        if "collector_scope" not in process:
            evidence.gap(RULE_IDS, "missing collector scope record", process=index)
        elif process["collector_scope"] != COLLECTOR_SCOPE:
            evidence.unverified("required collector scope differs", process=index)
        supported = process.get("vocabulary_supported")
        if supported is False:
            evidence.fail("vocabulary_coverage", "unsupported_fixture_text", process=index)
        elif supported is not True:
            evidence.gap(("vocabulary_coverage",), "missing fixture vocabulary record", process=index)
        if "lifecycle" in process:
            _check_terminal(evidence, process, index, budgets, resources, lifecycle_bound)
        else:
            evidence.gap(("lifecycle", "process_budgets"), "missing terminal record", process=index)
        rows_by_workload = {name: [] for name in specs}
        for row in process.get("samples", []):
            name = row.get("workload")
            if name not in specs:
                evidence.unverified(f"unknown workload {name!r}", process=index)
                continue
            rows_by_workload[name].append(row)
        workload_metrics = _mapping(process.get("workloads"))
        for name, (spec, mode) in specs.items():
            rows = rows_by_workload[name]
            workload_limits = _mapping(workload_budgets.get(name))
            if [(row.get("phase"), row.get("index")) for row in rows] != schedule:
                evidence.gap(SAMPLE_RULES, "incomplete/reordered sample schedule", process=index, workload=name)
            for row in rows:
                at = {"process": index, "workload": name, "phase": row.get("phase"), "sample": row.get("index")}
                if _check_sample(evidence, row, at, spec, mode, limits, workload_limits):
                    regions = [dict(region, text=_quality_text(region["text"]))
                               if isinstance(region.get("text"), str) else region for region in row["regions"]]
                    signature = json.dumps([regions, row.get("groups")], sort_keys=True, ensure_ascii=False)
                    if name in stable and signature != stable[name]:
                        evidence.fail("stability", "instability", **at)
                    stable.setdefault(name, signature)
                metrics = _mapping(row.get("metrics"))
                missing = [key for key in CRITERIA["required_sample_metrics"]
                           if not number(metrics.get(key), positive=key in POSITIVE_SAMPLE_METRICS)]
                if missing:
                    evidence.gap(SAMPLE_METRIC_RULES, f"missing sample metrics: {', '.join(missing)}", **at)
                if row.get("phase") == "sample" and number(metrics.get("elapsed_ms")):
                    times[name].append(metrics["elapsed_ms"])
                    for stage in stages:
                        if number(metrics.get(stage)):
                            stage_samples[name][stage].append(metrics[stage])
                    for key in ("detector_width", "detector_height", "detector_tensor_bytes", "mapped_bytes"):
                        if number(metrics.get(key)):
                            peak(name, key, metrics[key])
            workload = _mapping(workload_metrics.get(name))
            for key in CRITERIA["required_workload_metrics"]:
                value = workload.get(key)
                valid = (number(abs(value)) if key == "rust_growth_bytes"
                         and isinstance(value, (int, float)) and not isinstance(value, bool) else number(value))
                if not valid:
                    evidence.gap(("workload_budgets",), f"missing workload metric {key}", process=index, workload=name)
                    continue
                peak(name, key, value)
                limit = workload_limits.get(key)
                if number(limit) and value > limit:
                    evidence.fail("workload_budgets", "ceiling", process=index, workload=name, metric=key,
                                  observed=value, limit=limit)
            measured = [row["metrics"]["elapsed_ms"] for row in rows
                        if row.get("phase") == "sample" and number(_mapping(row.get("metrics")).get("elapsed_ms"))]
            if measured:
                for key, value in (("p50_ms", percentile(measured, .5)), ("p95_ms", percentile(measured, .95)), ("max_ms", max(measured))):
                    limit = workload_limits.get(key)
                    if number(limit) and value > limit:
                        evidence.fail("workload_budgets", "ceiling", process=index, workload=name, metric=key,
                                      observed=value, limit=limit)
    for name, values in times.items():
        summary = {}
        if values:
            summary = {"p50_ms": percentile(values, .5), "p95_ms": percentile(values, .95), "max_ms": max(values)}
        summary["samples"] = len(values)
        if values:
            summary["stages"] = {stage: {"p50_ms": percentile(samples, .5), "p95_ms": percentile(samples, .95), "max_ms": max(samples)}
                                 for stage, samples in stage_samples[name].items() if samples}
        summary["observed_max"] = observed_max[name]
        evidence.metrics["workloads"][name] = summary
    return evidence


def policy_failures(report, repository_root):
    errors = []
    binding = report.get("binding", {})
    source_revision = binding.get("source_revision")
    try:
        if not git_hex(source_revision):
            raise ValueError("invalid source revision")
        if git_bytes(repository_root, "rev-parse", "--verify", f"{source_revision}^{{commit}}").decode("ascii").strip() != source_revision:
            raise ValueError("source revision is not the exact commit")
        if git_bytes(repository_root, "rev-parse", f"{source_revision}^{{tree}}").decode("ascii").strip() != binding.get("source_tree"):
            errors.append("implementation source tree differs from Git commit authority")
    except (ValueError, UnicodeError) as error:
        errors.append(f"implementation Git authority unavailable/invalid: {error}")
    for key, hash_key in (("budgets", "budget_sha256"), ("host_policy", "host_policy_sha256")):
        evidence = report.get("policy_inputs", {}).get(key, {})
        try:
            raw = base64.b64decode(evidence["base64"], validate=True)
            digest = hashlib.sha256(raw).hexdigest()
            blob = hashlib.sha1(b"blob " + str(len(raw)).encode("ascii") + b"\0" + raw).hexdigest()
            if digest != evidence.get("sha256") or digest != report.get(hash_key) or blob != evidence.get("git_blob"):
                errors.append(f"frozen {key} byte/hash/blob mismatch")
            revision, relative = evidence.get("revision"), evidence.get("path")
            if revision != binding.get("frozen_revision") or not git_hex(revision):
                raise ValueError("frozen revision mismatch")
            if not _safe_relative(relative):
                raise ValueError("missing/unsafe repository-relative policy path")
            if git_bytes(repository_root, "rev-parse", "--verify", f"{revision}^{{commit}}").decode("ascii").strip() != revision:
                raise ValueError("frozen revision is not the exact commit")
            if not git_hex(source_revision):
                raise ValueError("invalid implementation source revision")
            git_bytes(repository_root, "merge-base", "--is-ancestor", revision, source_revision)
            entry = git_bytes(repository_root, "ls-tree", "-z", revision, "--", relative)
            metadata, name = entry.rstrip(b"\0").split(b"\t")
            mode, kind, committed_blob = metadata.decode("ascii").split(" ")
            if name.decode("utf-8") != relative or mode not in ("100644", "100755") or kind != "blob":
                raise ValueError("policy is not an exact regular-file Git tree entry")
            committed = git_bytes(repository_root, "cat-file", "blob", committed_blob)
            if committed_blob != evidence.get("git_blob") or committed != raw:
                errors.append(f"frozen {key} differs from historical Git tree/blob authority")
            expected = strict_json(committed.decode("utf-8"))
            if key == "budgets":
                expected["frozen_revision"] = revision
            if expected != report.get(key):
                errors.append(f"embedded {key} differs from frozen input bytes")
        except (ValueError, TypeError, KeyError, UnicodeError) as error:
            errors.append(f"frozen {key} Git authority unavailable/malformed: {error}")
    return errors


def descriptor_failures(report):
    """The real Rust binary/validator owns catalog normalization, not Python."""
    errors, described = [], {}
    binding = report.get("binding", {})
    for record in report.get("commands", []):
        process, command = record.get("process", {}), record.get("command", [])
        if "--describe-profile" not in command:
            continue
        if (process.get("exit_code") != 0 or process.get("cleanup_ok") is not True
                or process.get("timed_out") is not False or process.get("output_limited") is not False
                or process.get("launch_error") is not None):
            errors.append("compiled descriptor command failed")
            continue
        try:
            description = strict_json(process["stdout"])
            identity, build = description["identity"], description["build"]
            profile = identity["profile_id"]
            if profile in described or command[command.index("--profile") + 1] != profile:
                errors.append("duplicate/mismatched compiled descriptor selection")
            described[profile] = identity
            if any(build.get(key) != binding.get(key) for key in BUILD_FIELDS):
                errors.append("compiled catalog/executable/OpenCV/build provenance identity mismatch")
        except (ValueError, TypeError, KeyError, IndexError):
            errors.append("malformed compiled descriptor observation")
    if described != report.get("identities"):
        errors.append("execution identities differ from bound compiled descriptor authority")
    return errors


def identity_failures(profile, identity, report_version):
    """Execution descriptor shape; schema 1 adds the historical ``accepted`` fact."""
    if not isinstance(identity, dict) or not identity.get("execution_sha256") or not identity.get("resources"):
        return ["incomplete execution identity"]
    errors = []
    historical = report_version == 1
    if (set(identity) != DESCRIPTOR_FIELDS | ({"accepted"} if historical else set())
            or (historical and not isinstance(identity.get("accepted"), bool))):
        errors.append("execution identity fields differ from the report schema")
    resources, limits = identity["resources"], identity.get("limits")
    if (identity.get("profile_id") != profile or not _sha256_hex(identity["execution_sha256"])
            or not isinstance(resources, list) or not all(isinstance(resource, dict) for resource in resources)
            or {resource.get("kind") for resource in resources} not in ({"detector", "recognizer"}, {"detector", "recognizer", "auxiliary"})
            or any(not _safe_relative(resource.get("relative_path")) or not number(resource.get("bytes"), positive=True)
                   or not _sha256_hex(resource.get("sha256")) for resource in resources)
            or not isinstance(limits, dict) or not number(limits.get("tensor_bytes"), positive=True)
            or any(limits.get(axis) is not None and not number(limits.get(axis), positive=True)
                   for axis in ("detector_width", "detector_height"))):
        errors.append("malformed execution/resource identity")
    return errors


def _provenance(report):
    """Publishable scope/evidence facts; never host IDs, commands, paths or environment."""
    def text(row, key):
        value = row.get(key)
        return value if isinstance(value, str) else None

    def numeric(row, key):
        value = row.get(key)
        return value if number(value) else None

    binding = _mapping(report.get("binding"))
    scope = {key: numeric(binding, key) if key in ("intra_op", "inter_op") else text(binding, key)
             for key in ("target", "provider", "runtime", "intra_op", "inter_op", "os", "cpu",
                         "cohort_id", "host_approval")}
    if scope["target"] not in CRITERIA["release_targets"]:
        scope["target"] = None
    scope["criteria_id"] = text(report, "criteria_id")
    scope["measured_source"] = {"revision": text(binding, "source_revision"), "tree": text(binding, "source_tree")}
    policies = {}
    for key in ("budgets", "host_policy"):
        policy = _mapping(_mapping(report.get("policy_inputs")).get(key))
        if policy:
            policies[key] = {"path": policy.get("path") if _safe_relative(policy.get("path")) else None,
                             **{field: text(policy, field) for field in ("revision", "git_blob", "sha256")}}
    version, dirty = report.get("schema_version"), binding.get("build_source_dirty")
    corpora, limits = report.get("corpora"), report.get("process_limits")
    evidence = {
        "report_schema_version": version if type(version) is int and version in COMPLETE_EXECUTION else None,
        "execution": text(report, "execution"),
        "build": {"source_revision": text(binding, "build_source_revision"), "source_tree": text(binding, "build_source_tree"),
                  "source_dirty": dirty if type(dirty) is bool else None, "compiler": text(binding, "build_compiler"),
                  "opencv": text(binding, "opencv"), "executable_sha256": text(binding, "executable_sha256"),
                  "catalog_sha256": text(binding, "catalog_sha256")},
        **{key: text(binding, key) for key in ("runtime_sha256", "criteria_sha256", "fixtures_sha256", "frozen_revision")},
        "budget_sha256": text(report, "budget_sha256"),
        "host_policy_sha256": text(report, "host_policy_sha256"),
        "policies": policies,
        "corpora": [{key: text(_mapping(corpus), key) for key in ("profile", "sha256")} for corpus in corpora]
                   if isinstance(corpora, list) else None,
        "process_limits": {key: numeric(limits, key) for key in
                           ("timeout_seconds", "output_limit_bytes", "cleanup_seconds", "lifecycle_bound_ms")}
                          if isinstance(limits, dict) else None,
    }
    return scope, evidence


def _rules(evidence, *, executed, unverified, incomplete, reference):
    gaps, rules = evidence.gap_rules(), []
    for rule, category in RULES:
        findings, reason = evidence.findings[rule], None
        if not executed:
            outcome, reason = "not_run", "no_retained_execution"
        elif reference and rule == "relative_latency":
            outcome, reason = "not_run", "comparison_reference"
        elif unverified:
            outcome, reason = "incomplete", "unverified_evidence"
        elif findings:
            outcome = "failed"
        elif incomplete or rule in gaps:
            outcome, reason = "incomplete", "incomplete_evidence"
        else:
            outcome = "passed"
        entry = {"rule_id": rule, "category": category, "outcome": outcome}
        if reason:
            entry["reason"] = reason
        entry["findings"] = findings
        rules.append(entry)
    return rules


def _evaluate(report, *, evidence_root, repository_root):
    if not isinstance(report, dict) or not isinstance(report.get("binding"), dict):
        raise TypeError("report/binding is not a JSON object")
    binding, fixtures = report["binding"], report.get("fixtures", [])
    version = report.get("schema_version")
    integrity, scope_gaps = [], []
    if type(version) is not int or version not in COMPLETE_EXECUTION:
        integrity.append("unsupported report schema version")
    elif report.get("execution") != COMPLETE_EXECUTION[version]:
        scope_gaps.append(f"campaign execution {report.get('execution')!r} is not a complete run")
    profiles, raw_errors = reconcile_raw(report, evidence_root)
    integrity.extend(validate_binding(binding))
    integrity.extend(policy_failures(report, repository_root))
    integrity.extend(descriptor_failures(report))
    if binding.get("fixtures_sha256") != hashlib.sha256(document_bytes({"fixtures": fixtures})).hexdigest():
        integrity.append("prepared fixture/oracle bytes differ from bound input")
    limits = _mapping(report.get("process_limits"))
    if not all(number(limits.get(key), positive=True) for key in ("timeout_seconds", "output_limit_bytes", "cleanup_seconds", "lifecycle_bound_ms")):
        integrity.append("missing explicit process/lifecycle deadline/output/cleanup bounds")
    budgets = report.get("budgets")
    if report.get("process_limits") != _mapping(budgets).get("process_limits"):
        integrity.append("process bounds differ from frozen budget policy")
    if report.get("diagnostic") is not False:
        integrity.append("diagnostic execution is never a complete evaluation")
    if report.get("source_changes"):
        integrity.append("uncommitted implementation source")
    projection = [{key: row.get(key) for key in ("id", "sha256", "byte_len", "width", "height", "regions")} for row in fixtures]
    if projection != frozen_corpus():
        integrity.append("corpus/oracles differ from the complete frozen G-004 v3 and compact UI suite")
    if report.get("corpora") != corpus_identities():
        integrity.append("frozen corpus manifest identities mismatch")
    if binding.get("criteria_sha256") != sha256(Path(__file__).with_name("criteria-v1.json")):
        integrity.append("criteria identity mismatch")
    cohort = {key: binding.get(key) for key in COHORT_FIELDS}
    if not any(row.get("cohort") == cohort and row.get("approval") == binding.get("host_approval") and row.get("approval")
               for row in _mapping(report.get("host_policy")).get("approved_cohorts", [])):
        integrity.append("host cohort absent from frozen release allowlist")
    for key in ("host_policy_sha256", "budget_sha256"):
        if not _sha256_hex(report.get(key)):
            integrity.append(f"missing policy identity {key}")
    workload_ids = [f"{row['id']}/{mode}" for row in fixtures for mode in ("singular", "grouped")]
    if not fixtures or len(workload_ids) != len(set(workload_ids)):
        integrity.append("missing/duplicate frozen corpus workloads")
    if report.get("preflight_failures"):
        integrity.append(f"campaign retained {len(report['preflight_failures'])} preflight/operational "
                         "failure(s); details stay in the private report")
    integrity.extend(validate_budgets(budgets, binding, workload_ids))
    reference_id, candidate_id = report.get("reference_profile"), report.get("candidate_profile")
    if not isinstance(reference_id, str) or not isinstance(candidate_id, str) or reference_id == candidate_id:
        raise ValueError("distinct reference and candidate profile IDs required")
    claimed = report.get("profiles", {})
    if set(claimed) - {reference_id, candidate_id}:
        integrity.append("retained process profiles include an undeclared attempt")
    identities = _mapping(report.get("identities"))
    lifecycle_bound = _mapping(_mapping(budgets).get("process_limits")).get("lifecycle_bound_ms")
    roles = {reference_id: "reference", candidate_id: "candidate"}
    observed = {}
    for profile in roles:
        identity = identities.get(profile)
        evidence = evaluate_profile(profiles.get(profile, []), profile, binding, fixtures, budgets, identity, lifecycle_bound)
        for message in identity_failures(profile, identity, version):
            evidence.unverified(message)
        observed[profile] = evidence
    for profile, index, message in raw_errors:
        if profile in observed:
            observed[profile].unverified(message, process=index)
        else:
            integrity.append(message if profile is None else f"{profile}: {message}")
    executed = {profile: bool(claimed.get(profile)) or bool(profiles.get(profile)) for profile in roles}
    reference, candidate = observed[reference_id], observed[candidate_id]
    if executed[candidate_id]:
        if not executed[reference_id] or reference.invalid or "relative_latency" in reference.gap_rules():
            candidate.gap(("relative_latency",), "comparison reference timing evidence is incomplete or unverified")
        elif "relative_latency" not in candidate.gap_rules():
            ratio = CRITERIA["fast_maximum_reference_ratio"]
            for name in workload_ids:
                for metric in ("p50_ms", "p95_ms"):
                    value = candidate.metrics["workloads"].get(name, {}).get(metric)
                    baseline = reference.metrics["workloads"].get(name, {}).get(metric)
                    if not number(value) or not number(baseline):
                        candidate.gap(("relative_latency",), f"missing comparison timing {metric}", workload=name)
                    elif not value <= ratio * baseline:
                        candidate.fail("relative_latency", "reference_ratio", workload=name, metric=metric,
                                       observed=value, reference=baseline, limit=ratio * baseline,
                                       maximum_ratio=ratio)
    errors = [{"kind": "integrity", "profile": None, "message": message} for message in integrity]
    errors += [{"kind": "incomplete", "profile": None, "message": message} for message in scope_gaps]
    entries = {}
    for profile, role in roles.items():
        evidence, identity = observed[profile], identities.get(profile)
        rules = _rules(evidence, executed=executed[profile], unverified=bool(integrity or evidence.invalid),
                       incomplete=bool(scope_gaps), reference=role == "reference")
        errors += [{"kind": "integrity", "profile": profile, **entry} for entry in evidence.invalid]
        errors += [{"kind": "incomplete", "profile": profile, **entry} for _, entry in evidence.gaps]
        errors += [{"kind": "technical", "profile": profile, "rule_id": rule["rule_id"],
                    "message": "technical contract failed"}
                   for rule in rules if rule["category"] == "technical" and rule["outcome"] == "failed"]
        valid_identity = not identity_failures(profile, identity, version)
        entries[profile] = {
            "role": role,
            "execution_sha256": identity["execution_sha256"] if valid_identity else None,
            "resources": [{key: resource[key] for key in ("kind", "relative_path", "bytes", "sha256")}
                          for resource in identity["resources"]] if valid_identity else [],
            "processes": evidence.processes,
            "rules": rules,
            "metrics": evidence.metrics,
        }
    scope, provenance = _provenance(report)
    return {"complete": not errors, "validation_errors": errors, "scope": scope, "evidence": provenance,
            "profiles": entries}


def evaluate(report, *, evidence_root=None, repository_root=ROOT):
    """One target scope's per-rule outcomes; ``complete`` means verified, complete evidence
    whose technical contracts held, never that every empirical rule passed."""
    try:
        return _evaluate(report, evidence_root=evidence_root, repository_root=repository_root)
    except (ValueError, TypeError, KeyError, IndexError, AttributeError, OSError, ArithmeticError, RecursionError) as error:
        return _malformed(_mapping(report), f"malformed/incomplete report: {_describe(error)}")


def _malformed(report, message):
    scope, provenance = _provenance(report)
    return {"complete": False, "validation_errors": [{"kind": "integrity", "profile": None, "message": message}],
            "scope": scope, "evidence": provenance, "profiles": {}}


def evaluation_bundle(evaluations):
    """Independent scopes; unknown report attribution does not assert missing-target execution."""
    covered = {evaluation["scope"]["target"] for evaluation in evaluations}
    missing = [target for target in CRITERIA["release_targets"] if target not in covered]
    return {"schema_version": EVALUATION_SCHEMA_VERSION, "evaluator_revision": evaluator_revision(),
            "complete": bool(evaluations) and all(evaluation["complete"] for evaluation in evaluations),
            "not_run_targets": None if None in covered else missing,
            "evaluations": evaluations}


def evaluate_reports(reports, *, evidence_roots, repository_root=ROOT):
    """Evaluate report files with their original evidence roots, in order."""
    reports, evidence_roots = list(reports), list(evidence_roots)
    if len(reports) != len(evidence_roots):
        raise ValueError("one original evidence root per report is required")
    evaluations = []
    for path, root in zip(reports, evidence_roots):
        try:
            raw = Path(path).read_bytes()
            report = strict_json(raw.decode("utf-8"))
        except (OSError, ValueError, UnicodeError) as error:
            evaluations.append(_malformed({}, f"report unreadable/malformed JSON: {_describe(error)}"))
            continue
        evaluation = evaluate(report, evidence_root=root, repository_root=repository_root)
        try:
            document_bytes({"evaluations": [evaluation]})
        except (ValueError, TypeError, ArithmeticError, RecursionError) as error:
            evaluation = _malformed({}, f"report result cannot be serialized: {_describe(error)}")
        evaluation["evidence"]["report_sha256"] = hashlib.sha256(raw).hexdigest()
        evaluations.append(evaluation)
    return evaluation_bundle(evaluations)
