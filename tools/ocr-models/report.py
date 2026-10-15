"""Recompute admission from raw Rust observations; supplied verdicts are never authority."""
from __future__ import annotations

import base64
import hashlib
import json
import math
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


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


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
    return json.loads(text, object_pairs_hook=unique,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"nonfinite JSON {value}")))


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
        result["parse_failure"] = str(error)
    return result


def evidence_path(root, relative):
    if not isinstance(relative, str):
        raise ValueError("missing relative evidence path")
    path = PurePosixPath(relative)
    if not path.parts or path.is_absolute() or any(part in (".", "..") for part in relative.split("/")) or "\\" in relative or ":" in relative:
        raise ValueError("unsafe relative evidence path")
    resolved = (root / path).resolve(strict=True)
    resolved.relative_to(root)
    if not resolved.is_file():
        raise ValueError("evidence path is not a regular file")
    return resolved


def reconcile_raw(report, evidence_root):
    errors, profiles, retained, retained_receipts = [], {}, set(), set()
    if evidence_root is None:
        return report.get("profiles", {}), ["original raw JSONL evidence root is required"]
    root = Path(evidence_root).resolve(strict=True)
    for profile, processes in report.get("profiles", {}).items():
        profiles[profile] = []
        for process in processes:
            prefix = f"{profile}/process {process.get('index', '?')}"
            evidence_kind = "raw JSONL"
            try:
                path = evidence_path(root, process.get("raw_path"))
                if path.suffix != ".jsonl" or path in retained:
                    raise ValueError("duplicate/non-JSONL raw evidence path")
                retained.add(path)
                evidence_kind = "process receipt"
                receipt_path = evidence_path(root, process.get("receipt_path"))
                if not receipt_path.name.endswith(".process.json") or receipt_path in retained_receipts:
                    raise ValueError("duplicate/invalid process receipt path")
                retained_receipts.add(receipt_path)
                receipt_bytes = receipt_path.read_bytes()
                receipt_sha256 = hashlib.sha256(receipt_bytes).hexdigest()
                if receipt_sha256 != process.get("receipt_sha256"):
                    errors.append(f"{prefix}: original process receipt digest mismatch")
                receipt = strict_json(receipt_bytes.decode("utf-8"))
                if set(receipt) != {"index", "process", "command"}:
                    raise ValueError("invalid process receipt fields")
                original = parse_process(path, receipt["process"], receipt["command"],
                                         receipt["index"], evidence_root=root)
                if original["index"] != receipt["index"]:
                    errors.append(f"{prefix}: raw observation/process receipt index mismatch")
                original.update(receipt_path=receipt_path.relative_to(root).as_posix(),
                                receipt_sha256=receipt_sha256)
                if original.get("raw_sha256") != process.get("raw_sha256"):
                    errors.append(f"{prefix}: original raw JSONL digest mismatch")
                differing = sorted(key for key in set(original) | set(process)
                                   if key not in original or key not in process or original[key] != process[key])
                if differing:
                    errors.append(f"{prefix}: original raw JSONL projection mismatch: {', '.join(differing)}")
                profiles[profile].append(original)
            except (ValueError, TypeError, OSError) as error:
                errors.append(f"{prefix}: original {evidence_kind} unavailable/invalid: {error}")
                profiles[profile].append(process)
    if {path.resolve() for path in root.rglob("*.jsonl")} != retained:
        errors.append("original raw JSONL attempt set differs from retained report")
    if {path.resolve() for path in root.rglob("*.process.json")} != retained_receipts:
        errors.append("original process receipt attempt set differs from retained report")
    return profiles, errors


def git_bytes(repository_root, *arguments):
    try:
        result = subprocess.run(["git", "--no-replace-objects", "-C", str(repository_root), *arguments],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"Git authority unavailable: {error}") from error
    if result.returncode != 0:
        raise ValueError(f"Git authority failed ({' '.join(arguments)}): {result.stderr.decode('utf-8', errors='replace').strip()}")
    return result.stdout


def git_hex(value):
    return isinstance(value, str) and len(value) == 40 and all(c in "0123456789abcdef" for c in value)


def number(value, *, positive=False) -> bool:
    return (isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value) and (value > 0 if positive else value >= 0))


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


def quality(observation, expected, mode):
    errors = []
    if observation.get("status") != "Ok":
        errors.append("recognition status is not Ok")
    if observation.get("source_correlation") is not True:
        errors.append("source correlation failed")
    source = observation.get("source")
    source_fields = ("stamp", "transform", "effective_region", "backend", "output_space", "effective_zones")
    if (not isinstance(source, dict) or any(not source.get(key) for key in source_fields)
            or observation.get("result_source") != source):
        errors.append("raw source/result correlation identity mismatch")
    actual = observation.get("regions")
    if not isinstance(actual, list):
        return errors + ["missing raw regions"]
    if len(actual) != len(expected):
        errors.append(f"region count {len(actual)} != {len(expected)}")
    for index, region in enumerate(actual):
        confidence = region.get("confidence")
        if not number(confidence) or confidence > 1:
            errors.append(f"region {index}: nonfinite/out-of-range confidence")
        quad = region.get("quad")
        if (not isinstance(quad, list) or len(quad) != 4 or any(
                not isinstance(point, list) or len(point) != 2
                or not all(number(v) and v <= 1 for v in point) for point in quad)):
            errors.append(f"region {index}: invalid source-relative quad")
            continue
        if index >= len(expected):
            if number(confidence) and confidence >= 0.5:
                errors.append(f"region {index}: unmatched detection >=0.5")
            continue
        oracle = expected[index]
        text = region.get("text")
        if not isinstance(text, str) or unicodedata.normalize("NFC", text.strip()) != oracle["text_nfc"]:
            errors.append(f"region {index}: exact text/order failed")
        target = oracle["source_relative_quad"]
        intersection = _area(_intersection(quad, target))
        union = _area(quad) + _area(target) - intersection
        if union <= 0 or intersection / union < 0.5:
            errors.append(f"region {index}: IoU below 0.50")
        for axis in (0, 1):
            center = (min(p[axis] for p in quad) + max(p[axis] for p in quad)) / 2
            target_center = (min(p[axis] for p in target) + max(p[axis] for p in target)) / 2
            if abs(center - target_center) > 0.025:
                errors.append(f"region {index}: center delta axis {axis} exceeds 0.025")
    if mode == "grouped" and observation.get("groups") != [list(range(len(actual))), list(range(len(actual)))]:
        errors.append("grouped candidate memberships/order failed")
    return errors


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
        value = binding.get(key, "")
        if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
            errors.append(f"invalid binding {key}")
    if binding.get("target") not in CRITERIA["release_targets"]:
        errors.append("wrong release target")
    if binding.get("provider") != "cpu" or binding.get("intra_op") != 1 or binding.get("inter_op") != 1:
        errors.append("CPU thread-one policy missing")
    if binding.get("qualified_host") is not True or not isinstance(binding.get("host_approval"), str) or not binding["host_approval"].strip():
        errors.append("host is not qualified/approved")
    # The current experimental macOS 27 host is never release evidence.
    if binding.get("target") == "aarch64-apple-darwin" and str(binding.get("os", "")).startswith(("macOS 27", "27.")):
        errors.append("macOS 27 is not an approved release host")
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


def validate_profile(processes, profile_id, binding, fixtures, budgets, expected_identity):
    errors, summaries = [], {}
    specs = {f"{row['id']}/{mode}": (row, mode) for row in fixtures for mode in ("singular", "grouped")}
    if len(processes) < 3:
        errors.append("fewer than three fresh processes")
    indexes, pids = set(), set()
    all_times = {name: [] for name in specs}
    stages = [key for key in CRITERIA["required_sample_metrics"] if key.endswith("_ms") and key != "elapsed_ms"]
    stage_samples = {name: {key: [] for key in stages} for name in specs}
    stable = {}
    for process in processes:
        prefix = f"process {process.get('index', '?')}"
        if process.get("parse_failure") or process.get("close_fault") is not None or any(row.get("type") == "failure" for row in process.get("raw_records", [])):
            errors.append(f"{prefix}: preserved Rust/apparatus failure: {process.get('parse_failure') or process.get('raw_records') or process.get('close_fault')}")
        if not isinstance(process.get("command"), list) or not process["command"]:
            errors.append(f"{prefix}: missing exact reproduction command")
        if process.get("index") in indexes or process.get("pid") in pids or not isinstance(process.get("pid"), int):
            errors.append(f"{prefix}: duplicate/missing fresh process identity")
        indexes.add(process.get("index")); pids.add(process.get("pid"))
        operational = process.get("process", {})
        if (operational.get("exit_code") != 0 or operational.get("timed_out") is not False
                or operational.get("output_limited") is not False or operational.get("cleanup_ok") is not True
                or operational.get("launch_error") is not None):
            errors.append(f"{prefix}: bounded child failed")
        if process.get("binding") != binding or process.get("profile") != profile_id:
            errors.append(f"{prefix}: identity/cohort mismatch")
        if process.get("identity") != expected_identity:
            errors.append(f"{prefix}: execution/resource identity mismatch")
        if any(process.get("build", {}).get(key) != binding.get(key) for key in BUILD_FIELDS):
            errors.append(f"{prefix}: actual compiled build facts mismatch")
        if process.get("collector_scope") != {"rust_accounting": "SDK plus raw observation construction/serialization",
                                             "native_peak_rss": "entire fresh process including cold initialization, workloads and lifecycle"}:
            errors.append(f"{prefix}: required collector scope missing")
        if process.get("vocabulary_supported") is not True:
            errors.append(f"{prefix}: fixture vocabulary coverage missing")
        if process.get("target") != binding.get("target") or process.get("opencv") != binding.get("opencv"):
            errors.append(f"{prefix}: actual target/OpenCV mismatch")
        if process.get("runtime") != binding.get("runtime") or process.get("provider") != "cpu":
            errors.append(f"{prefix}: actual runtime/provider mismatch")
        for key in PROCESS_METRICS:
            value = process.get("metrics", {}).get(key)
            if not number(value, positive=key != "auxiliary_bytes"):
                errors.append(f"{prefix}: missing process metric {key}")
            elif key in BUDGET_PROCESS_METRICS and number((budgets or {}).get("process", {}).get(key)) and value > budgets["process"][key]:
                errors.append(f"{prefix}: absolute process budget {key}")
        if any(process.get("lifecycle", {}).get(key) != value for key, value in CRITERIA["required_lifecycle"].items()):
            errors.append(f"{prefix}: typed lifecycle failed")
        if process.get("lifecycle", {}).get("initialization_cancelled") != "Cancelled":
            errors.append(f"{prefix}: cancelled initialization outcome missing/failed")
        lifecycle_bound = (budgets or {}).get("process_limits", {}).get("lifecycle_bound_ms")
        for kind in ("cancelled", "deadline"):
            detail = process.get("lifecycle_detail", {}).get(kind, {})
            if (detail.get("scope") != "admitted-before-native-entry"
                    or any(detail.get(key) is not True for key in ("admitted", "termination_issued", "worker_joined"))
                    or detail.get("cleanup_completions") != 1 or detail.get("bound_ms") != lifecycle_bound
                    or not number(detail.get("return_ms")) or not number(lifecycle_bound, positive=True)
                    or detail["return_ms"] > lifecycle_bound):
                errors.append(f"{prefix}: admitted {kind} settlement/collector contract failed")
        resources = (expected_identity or {}).get("resources", [])
        expected_model_bytes = sum(resource.get("bytes", 0) for resource in resources if resource.get("kind") in ("detector", "recognizer"))
        expected_auxiliary_bytes = sum(resource.get("bytes", 0) for resource in resources if resource.get("kind") == "auxiliary")
        if process.get("metrics", {}).get("model_bytes") != expected_model_bytes or process.get("metrics", {}).get("auxiliary_bytes") != expected_auxiliary_bytes:
            errors.append(f"{prefix}: resource byte metrics differ from execution identity")
        rows_by_workload = {name: [] for name in specs}
        for observation in process.get("samples", []):
            name = observation.get("workload")
            if name not in specs:
                errors.append(f"{prefix}: unknown workload {name}")
                continue
            rows_by_workload[name].append(observation)
        workload_metrics = process.get("workloads", {})
        for name, (spec, mode) in specs.items():
            rows = rows_by_workload[name]
            expected_schedule = [(phase, i) for phase, count in (("warmup", 2), ("sample", 20)) for i in range(count)]
            if [(r.get("phase"), r.get("index")) for r in rows] != expected_schedule:
                errors.append(f"{prefix}/{name}: incomplete/reordered sample schedule")
            for row in rows:
                errors.extend(f"{prefix}/{name}/{row.get('phase')}/{row.get('index')}: {e}" for e in quality(row, spec["regions"], mode))
                signature = json.dumps([row.get("regions"), row.get("groups")], sort_keys=True, ensure_ascii=False)
                if name in stable and signature != stable[name]:
                    errors.append(f"{prefix}/{name}: same-implementation instability")
                stable.setdefault(name, signature)
                metrics = row.get("metrics", {})
                for key in CRITERIA["required_sample_metrics"]:
                    if not number(metrics.get(key), positive=key in ("detector_width", "detector_height", "detector_tensor_bytes", "detector_runs", "mapped_bytes")):
                        errors.append(f"{prefix}/{name}: missing sample metric {key}")
                if metrics.get("detector_runs") != 1:
                    errors.append(f"{prefix}/{name}: detector run contract")
                if metrics.get("mapped_bytes") != spec["width"] * spec["height"] * 4:
                    errors.append(f"{prefix}/{name}: source mapping byte contract")
                width, height = metrics.get("detector_width"), metrics.get("detector_height")
                if number(width, positive=True) and number(height, positive=True):
                    if metrics.get("detector_tensor_bytes") != width * height * 3 * 4:
                        errors.append(f"{prefix}/{name}: actual detector shape/bytes disagree")
                    limits = (expected_identity or {}).get("limits", {})
                    for axis, value in (("width", width), ("height", height)):
                        limit = limits.get(f"detector_{axis}")
                        if limit is not None and (not number(limit, positive=True) or value > limit):
                            errors.append(f"{prefix}/{name}: detector {axis} exceeds profile")
                    tensor_limit = limits.get("tensor_bytes")
                    if not number(tensor_limit, positive=True) or metrics.get("detector_tensor_bytes", 0) > tensor_limit:
                        errors.append(f"{prefix}/{name}: profile tensor byte ceiling missing/exceeded")
                if metrics.get("recognizer_runs") != math.ceil(len(row.get("regions", [])) / 6):
                    errors.append(f"{prefix}/{name}: recognition batching contract")
                if row.get("phase") == "sample" and number(metrics.get("elapsed_ms")):
                    all_times[name].append(metrics["elapsed_ms"])
                    for stage in stages:
                        if number(metrics.get(stage)):
                            stage_samples[name][stage].append(metrics[stage])
                for key in ("mapped_bytes", "detector_tensor_bytes"):
                    limit = (budgets or {}).get("workloads", {}).get(name, {}).get(key)
                    if number(metrics.get(key)) and number(limit) and metrics[key] > limit:
                        errors.append(f"{prefix}/{name}: absolute budget {key}")
            wm = workload_metrics.get(name, {})
            for key in CRITERIA["required_workload_metrics"]:
                value = wm.get(key)
                valid = (isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)) if key == "rust_growth_bytes" else number(value)
                if not valid:
                    errors.append(f"{prefix}/{name}: missing workload metric {key}")
                else:
                    limit = (budgets or {}).get("workloads", {}).get(name, {}).get(key)
                    if number(limit) and value > limit:
                        errors.append(f"{prefix}/{name}: absolute budget {key}")
            measured = [r["metrics"]["elapsed_ms"] for r in rows if r.get("phase") == "sample" and number(r.get("metrics", {}).get("elapsed_ms"))]
            if measured:
                for key, value in (("p50_ms", percentile(measured, .5)), ("p95_ms", percentile(measured, .95)), ("max_ms", max(measured))):
                    limit = (budgets or {}).get("workloads", {}).get(name, {}).get(key)
                    if number(limit) and value > limit:
                        errors.append(f"{prefix}/{name}: absolute budget {key}")
    for name, values in all_times.items():
        if values:
            summaries[name] = {"p50_ms": percentile(values, .5), "p95_ms": percentile(values, .95), "max_ms": max(values), "samples": len(values)}
            summaries[name]["stages"] = {stage: {"p50_ms": percentile(samples, .5), "p95_ms": percentile(samples, .95), "max_ms": max(samples)}
                                        for stage, samples in stage_samples[name].items() if samples}
    return errors, summaries


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
            if not isinstance(relative, str) or not PurePosixPath(relative).parts or PurePosixPath(relative).is_absolute() or any(part in ("", ".", "..") for part in relative.split("/")) or "\\" in relative or ":" in relative:
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


def _classify(report, *, evidence_root, repository_root):
    """Return only recomputed policy conclusions; a failed candidate has no rating."""
    binding, fixtures = report.get("binding", {}), report.get("fixtures", [])
    profiles, raw_errors = reconcile_raw(report, evidence_root)
    report = dict(report, profiles=profiles)
    errors = validate_binding(binding) + raw_errors
    errors.extend(policy_failures(report, repository_root))
    errors.extend(descriptor_failures(report))
    if binding.get("fixtures_sha256") != hashlib.sha256(document_bytes({"fixtures": fixtures})).hexdigest():
        errors.append("prepared fixture/oracle bytes differ from bound input")
    limits = report.get("process_limits", {})
    if not all(number(limits.get(key), positive=True) for key in ("timeout_seconds", "output_limit_bytes", "cleanup_seconds", "lifecycle_bound_ms")):
        errors.append("missing explicit process/lifecycle deadline/output/cleanup bounds")
    if limits != (report.get("budgets") or {}).get("process_limits"):
        errors.append("process bounds differ from frozen budget policy")
    if report.get("diagnostic") is True:
        errors.append("diagnostic execution never admits or rates a model")
    if report.get("source_changes"):
        errors.append("uncommitted implementation source")
    expected_corpus = frozen_corpus()
    projection = [{key: row.get(key) for key in ("id", "sha256", "byte_len", "width", "height", "regions")} for row in fixtures]
    if projection != expected_corpus:
        errors.append("corpus/oracles differ from the complete frozen G-004 v3 and compact UI suite")
    if report.get("corpora") != corpus_identities():
        errors.append("frozen corpus manifest identities mismatch")
    if binding.get("criteria_sha256") != sha256(Path(__file__).with_name("criteria-v1.json")):
        errors.append("criteria identity mismatch")
    cohort = {key: binding.get(key) for key in COHORT_FIELDS}
    if not any(row.get("cohort") == cohort and row.get("approval") == binding.get("host_approval") and row.get("approval")
               for row in report.get("host_policy", {}).get("approved_cohorts", [])):
        errors.append("host cohort absent from frozen release allowlist")
    for key in ("host_policy_sha256", "budget_sha256"):
        if not isinstance(report.get(key), str) or len(report[key]) != 64:
            errors.append(f"missing policy identity {key}")
    workload_ids = [f"{row['id']}/{mode}" for row in fixtures for mode in ("singular", "grouped")]
    if not fixtures or len(workload_ids) != len(set(workload_ids)):
        errors.append("missing/duplicate frozen corpus workloads")
    errors.extend(report.get("preflight_failures", []))
    errors.extend(validate_budgets(report.get("budgets"), binding, workload_ids))
    reference_id, candidate_id = report.get("reference_profile"), report.get("candidate_profile")
    identities = report.get("identities", {})
    if not reference_id or not candidate_id or reference_id == candidate_id:
        errors.append("distinct reference and candidate identities required")
    if set(report.get("profiles", {})) != {reference_id, candidate_id}:
        errors.append("retained process profiles differ from the exact reference/candidate attempt set")
    if identities.get(reference_id, {}).get("accepted") is not True:
        errors.append("reference is not an accepted profile")
    if identities.get(candidate_id, {}).get("accepted") is not False:
        errors.append("candidate declaration missing")
    summaries = {}
    for profile in (reference_id, candidate_id):
        identity = identities.get(profile)
        if not isinstance(identity, dict) or not identity.get("execution_sha256") or not identity.get("resources"):
            errors.append(f"{profile}: incomplete execution identity")
        elif (identity.get("profile_id") != profile or len(identity["execution_sha256"]) != 64
              or set(resource.get("kind") for resource in identity["resources"]) not in ({"detector", "recognizer"}, {"detector", "recognizer", "auxiliary"})
              or any(not resource.get("relative_path") or not number(resource.get("bytes"), positive=True)
                     or not isinstance(resource.get("sha256"), str) or len(resource["sha256"]) != 64
                     for resource in identity["resources"])
              or not isinstance(identity.get("limits"), dict)):
            errors.append(f"{profile}: malformed execution/resource identity")
        pe, ps = validate_profile(report.get("profiles", {}).get(profile, []), profile, binding, fixtures, report.get("budgets"), identity)
        errors.extend(f"{profile}: {e}" for e in pe)
        summaries[profile] = ps
    if errors:
        return {"admitted": False, "target_qualified": False, "reference": None, "candidate": None, "failures": errors, "summaries": summaries}
    fast = all(summaries[candidate_id][name][metric] <= .8 * summaries[reference_id][name][metric]
               for name in workload_ids for metric in ("p50_ms", "p95_ms"))
    return {"admitted": False, "target_qualified": True, "reference": "reference", "candidate": "fast" if fast else "balanced", "failures": [], "summaries": summaries}


def classify(report, *, evidence_root=None, repository_root=ROOT):
    """Original JSONL and Git objects are required; supplied labels never qualify."""
    try:
        return _classify(report, evidence_root=evidence_root, repository_root=repository_root)
    except (ValueError, TypeError, KeyError, IndexError, AttributeError, OSError) as error:
        return {"admitted": False, "target_qualified": False, "reference": None, "candidate": None,
                "failures": [f"malformed/incomplete report: {error}"], "summaries": {}}


def admission(reports, *, evidence_roots=None, repository_root=ROOT):
    """Two approved target reports establish eligibility; this never mutates support."""
    roots = list(evidence_roots) if evidence_roots is not None else [None] * len(reports)
    if len(roots) != len(reports):
        return {"admitted": False, "failures": ["one original evidence root per report is required"], "targets": {}}
    verdicts = [classify(report, evidence_root=root, repository_root=repository_root)
                for report, root in zip(reports, roots)]
    errors = [f"report {index}: {failure}" for index, verdict in enumerate(verdicts) for failure in verdict["failures"]]
    targets = [report.get("binding", {}).get("target") for report in reports]
    if len(reports) != 2 or set(targets) != set(CRITERIA["release_targets"]):
        errors.append("independent approved Apple and Windows target reports required")
    if len(reports) == 2:
        first, second = reports
        for key in ("candidate_profile", "reference_profile", "identities", "corpora", "criteria_id"):
            if first.get(key) != second.get(key):
                errors.append(f"cross-target frozen identity mismatch: {key}")
        for key in ("source_revision", "source_tree", "catalog_sha256", "criteria_sha256"):
            if first.get("binding", {}).get(key) != second.get("binding", {}).get(key):
                errors.append(f"cross-target implementation/criteria mismatch: {key}")
    return {"admitted": not errors and all(verdict["target_qualified"] for verdict in verdicts),
            "failures": errors, "targets": dict(zip(targets, verdicts))}


def unrated(*, already_accepted: bool):
    """Legacy presentation metadata is not a route to candidate admission."""
    return {"admitted": False, "code": "unrated" if already_accepted else None}
