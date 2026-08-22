#!/usr/bin/env python3
"""Run one frozen G-004 OCR candidate and emit privacy-reviewed evidence."""

from __future__ import annotations

import argparse
import copy
import ctypes
import hashlib
import importlib.metadata
import json
import math
import os
import platform
import statistics
import sys
import time
import unicodedata
from pathlib import Path
from typing import Any

import cv2
import numpy as np
import onnxruntime as ort
from PIL import __version__ as pillow_version
import rapidocr
from rapidocr.ch_ppocr_det import TextDetector
from rapidocr.ch_ppocr_rec import TextRecInput, TextRecognizer
from rapidocr.main import DEFAULT_CFG_PATH
from rapidocr.utils.parse_parameters import ParseParams
from rapidocr.utils.process_img import get_rotate_crop_image
from shapely.geometry import Polygon

ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = ROOT / "docs" / "evidence" / "g-004"
FIXTURES = ROOT / "fixtures" / "ocr" / "g-004"
PRIVATE_EPHEMERA = (
    ROOT
    / ".rasen"
    / "changes"
    / "phase-3-g004-default-ocr-profile"
    / "ephemera"
)
PAIR_LIMIT = 64 * 1024 * 1024
WARMUP_PASSES = 2
MEASURED_PASSES = 10


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_package_digest(package_root: Path) -> dict[str, Any]:
    digest = hashlib.sha256()
    file_count = 0
    total_bytes = 0
    for path in sorted(
        (
            path
            for path in package_root.rglob("*")
            if path.is_file() and path.suffix in {".py", ".yaml", ".yml"}
        ),
        key=lambda path: path.relative_to(package_root).as_posix(),
    ):
        relative = path.relative_to(package_root).as_posix().encode("utf-8")
        content = path.read_bytes()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
        file_count += 1
        total_bytes += len(content)
    if file_count == 0:
        raise ValueError("RapidOCR package contains no digestible source files")
    return {
        "algorithm": "sha256(path_length_u64be || path_utf8 || content_length_u64be || content)",
        "included_suffixes": [".py", ".yaml", ".yml"],
        "file_count": file_count,
        "total_bytes": total_bytes,
        "sha256": digest.hexdigest(),
    }


def verify_environment() -> dict[str, Any]:
    requirements_path = EVIDENCE / "tool-requirements.txt"
    expected_python = None
    expected_packages: dict[str, str] = {}
    for line in requirements_path.read_text(encoding="utf-8").splitlines():
        if line.startswith("# Python "):
            expected_python = line.removeprefix("# Python ").split(";", 1)[0]
        elif line and not line.startswith("#"):
            name, version = line.split("==", 1)
            expected_packages[name] = version

    actual_python = platform.python_version()
    if expected_python is None or actual_python != expected_python:
        raise ValueError(
            f"Python version mismatch: expected {expected_python}, got {actual_python}"
        )

    actual_packages = {
        name: importlib.metadata.version(name) for name in expected_packages
    }
    mismatches = {
        name: {"expected": expected_packages[name], "actual": actual_packages[name]}
        for name in expected_packages
        if actual_packages[name] != expected_packages[name]
    }
    if mismatches:
        raise ValueError(f"evaluation package version mismatch: {mismatches}")

    if rapidocr.__file__ is None:
        raise ValueError("RapidOCR package root is unavailable")
    rapidocr_code = canonical_package_digest(
        Path(rapidocr.__file__).resolve().parent
    )

    return {
        "python": actual_python,
        "packages": actual_packages,
        "modules": {
            "onnxruntime": ort.__version__,
            "opencv": cv2.__version__,
            "pillow": pillow_version,
        },
        "onnxruntime_available_providers": ort.get_available_providers(),
        "rapidocr_code": rapidocr_code,
    }


def normalize_text(value: str) -> str:
    return unicodedata.normalize("NFC", value).strip()


def nearest_rank(values: list[float], percentile: float) -> float:
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def physical_memory_bytes() -> int | None:
    if sys.platform == "win32":
        class MemoryStatus(ctypes.Structure):
            _fields_ = [
                ("length", ctypes.c_ulong),
                ("memory_load", ctypes.c_ulong),
                ("total_physical", ctypes.c_ulonglong),
                ("available_physical", ctypes.c_ulonglong),
                ("total_page_file", ctypes.c_ulonglong),
                ("available_page_file", ctypes.c_ulonglong),
                ("total_virtual", ctypes.c_ulonglong),
                ("available_virtual", ctypes.c_ulonglong),
                ("available_extended_virtual", ctypes.c_ulonglong),
            ]

        status = MemoryStatus()
        status.length = ctypes.sizeof(status)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            return int(status.total_physical)
        return None

    page_size = os.sysconf("SC_PAGE_SIZE")
    pages = os.sysconf("SC_PHYS_PAGES")
    return int(page_size * pages)


def peak_resident_memory() -> dict[str, Any]:
    if sys.platform == "win32":
        class ProcessMemoryCounters(ctypes.Structure):
            _fields_ = [
                ("cb", ctypes.c_ulong),
                ("page_fault_count", ctypes.c_ulong),
                ("peak_working_set_size", ctypes.c_size_t),
                ("working_set_size", ctypes.c_size_t),
                ("quota_peak_paged_pool_usage", ctypes.c_size_t),
                ("quota_paged_pool_usage", ctypes.c_size_t),
                ("quota_peak_non_paged_pool_usage", ctypes.c_size_t),
                ("quota_non_paged_pool_usage", ctypes.c_size_t),
                ("pagefile_usage", ctypes.c_size_t),
                ("peak_pagefile_usage", ctypes.c_size_t),
            ]

        try:
            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            psapi = ctypes.WinDLL("psapi", use_last_error=True)
            kernel32.GetCurrentProcess.argtypes = []
            kernel32.GetCurrentProcess.restype = ctypes.c_void_p
            psapi.GetProcessMemoryInfo.argtypes = [
                ctypes.c_void_p,
                ctypes.POINTER(ProcessMemoryCounters),
                ctypes.c_ulong,
            ]
            psapi.GetProcessMemoryInfo.restype = ctypes.c_int

            counters = ProcessMemoryCounters()
            counters.cb = ctypes.sizeof(counters)
            process = kernel32.GetCurrentProcess()
            ctypes.set_last_error(0)
            if psapi.GetProcessMemoryInfo(
                process, ctypes.byref(counters), counters.cb
            ):
                return {
                    "status": "measured",
                    "bytes": int(counters.peak_working_set_size),
                    "source": "GetProcessMemoryInfo.PeakWorkingSetSize",
                }
            return {
                "status": "unavailable",
                "bytes": None,
                "reason": {
                    "kind": "windows_api_error",
                    "api": "GetProcessMemoryInfo",
                    "code": ctypes.get_last_error(),
                },
            }
        except (AttributeError, OSError) as error:
            return {
                "status": "unavailable",
                "bytes": None,
                "reason": {
                    "kind": "windows_api_unavailable",
                    "api": "GetProcessMemoryInfo",
                    "error_type": type(error).__name__,
                    "code": getattr(error, "winerror", None),
                },
            }

    try:
        import resource

        value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        return {
            "status": "measured",
            "bytes": int(value * 1024 if sys.platform.startswith("linux") else value),
            "source": "getrusage.ru_maxrss",
        }
    except (ImportError, OSError) as error:
        return {
            "status": "unavailable",
            "bytes": None,
            "reason": {
                "kind": "posix_resource_unavailable",
                "api": "getrusage",
                "error_type": type(error).__name__,
                "code": getattr(error, "errno", None),
            },
        }


def host_record() -> dict[str, Any]:
    return {
        "target": "aarch64-apple-darwin"
        if sys.platform == "darwin" and platform.machine().lower() in ("arm64", "aarch64")
        else "x86_64-pc-windows-msvc"
        if sys.platform == "win32" and platform.machine().lower() in ("amd64", "x86_64")
        else f"unsupported:{sys.platform}:{platform.machine()}",
        "os": platform.system(),
        "os_release": platform.release(),
        "os_version": platform.version(),
        "architecture": platform.machine(),
        "logical_cpu_count": os.cpu_count(),
        "physical_memory_bytes": physical_memory_bytes(),
    }


def resolve_output_paths(
    report: Path, raw_report: Path
) -> tuple[Path, Path]:
    report_path = report.resolve()
    raw_path = raw_report.resolve()
    private_root = PRIVATE_EPHEMERA.resolve()
    if not raw_path.is_relative_to(private_root):
        raise ValueError(
            "raw report must stay under the ignored G-004 private ephemera root"
        )
    if raw_path == report_path:
        raise ValueError("raw and sanitized report paths must be distinct")
    return report_path, raw_path


def load_fixture_inputs(fixture: dict[str, Any]) -> list[dict[str, Any]]:
    loaded = []
    for image_record in fixture["images"]:
        image_path = FIXTURES / image_record["file"]
        encoded_bytes = image_path.read_bytes()
        observed_sha256 = hashlib.sha256(encoded_bytes).hexdigest()
        if observed_sha256 != image_record["sha256"]:
            raise ValueError(f"fixture SHA-256 mismatch: {image_record['file']}")
        encoded = np.frombuffer(encoded_bytes, dtype=np.uint8)
        image = cv2.imdecode(encoded, cv2.IMREAD_COLOR)
        if image is None:
            raise ValueError(f"cannot decode fixture {image_record['file']}")
        if image.shape[:2] != (image_record["height"], image_record["width"]):
            raise ValueError(f"fixture dimensions mismatch: {image_record['file']}")
        loaded.append(
            {
                "record": image_record,
                "image": image,
                "consumed_bytes": len(encoded_bytes),
                "consumed_sha256": observed_sha256,
            }
        )
    return loaded


def configure_engine(candidate: dict[str, Any], model_root: Path) -> tuple[TextDetector, TextRecognizer, float]:
    detector_path = model_root / candidate["detector"]["relative_controlled_path"]
    recognizer_path = model_root / candidate["recognizer"]["relative_controlled_path"]

    started = time.perf_counter()
    cfg = ParseParams.load(DEFAULT_CFG_PATH)
    cfg = ParseParams.update_batch(
        cfg,
        {
            "Global.use_cls": False,
            "Global.text_score": 0.5,
            "Global.return_word_box": False,
            "Det.model_path": str(detector_path),
            "Det.limit_side_len": 736,
            "Det.limit_type": "min",
            "Det.std": [0.5, 0.5, 0.5],
            "Det.mean": [0.5, 0.5, 0.5],
            "Det.thresh": 0.3,
            "Det.box_thresh": 0.5,
            "Det.max_candidates": 1000,
            "Det.unclip_ratio": 1.6,
            "Det.use_dilation": True,
            "Det.score_mode": "fast",
            "Rec.model_path": str(recognizer_path),
            "Rec.rec_img_shape": [3, 48, 320],
            "Rec.rec_batch_num": 6,
            "EngineConfig.onnxruntime.intra_op_num_threads": 1,
            "EngineConfig.onnxruntime.inter_op_num_threads": 1,
            "EngineConfig.onnxruntime.enable_cpu_mem_arena": False,
            "EngineConfig.onnxruntime.use_cuda": False,
            "EngineConfig.onnxruntime.use_dml": False,
            "EngineConfig.onnxruntime.use_cann": False,
            "EngineConfig.onnxruntime.use_coreml": False,
        },
    )

    cfg.Det.engine_cfg = cfg.EngineConfig.onnxruntime
    cfg.Det.model_root_dir = model_root
    cfg.Rec.engine_cfg = cfg.EngineConfig.onnxruntime
    cfg.Rec.model_root_dir = model_root
    cfg.Rec.font_path = None
    detector = TextDetector(cfg.Det)
    recognizer = TextRecognizer(cfg.Rec)
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    return detector, recognizer, elapsed_ms


def verify_models(candidate: dict[str, Any], model_root: Path) -> list[dict[str, Any]]:
    verified = []
    total = 0
    for role in ("detector", "recognizer"):
        model = candidate[role]
        relative = Path(model["relative_controlled_path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"unsafe controlled model path for {role}")
        path = model_root / relative
        size = path.stat().st_size
        total += size
        if size != model["bytes"]:
            raise ValueError(f"{role} byte count mismatch")
        digest = sha256(path)
        if digest != model["sha256"]:
            raise ValueError(f"{role} SHA-256 mismatch")
        verified.append(
            {
                "role": role,
                "id": model["id"],
                "relative_controlled_path": model["relative_controlled_path"],
                "bytes": size,
                "sha256": digest,
            }
        )
    if total != candidate["total_model_bytes"] or total > PAIR_LIMIT:
        raise ValueError("candidate model pair byte limit or total mismatch")
    return verified


def session_values(values: Any) -> list[dict[str, Any]]:
    return [
        {"name": value.name, "shape": value.shape, "type": value.type}
        for value in values
    ]


def verify_session_metadata(
    candidate: dict[str, Any],
    detector: TextDetector,
    recognizer: TextRecognizer,
    fixture: dict[str, Any],
    verified_models: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    components = {"detector": detector, "recognizer": recognizer}
    verified_by_role = {model["role"]: model for model in verified_models}
    for role, component in components.items():
        model = candidate[role]
        session = component.session.session
        providers = session.get_providers()
        inputs = session_values(session.get_inputs())
        outputs = session_values(session.get_outputs())
        if providers != ["CPUExecutionProvider"]:
            raise ValueError(f"{role} did not bind CPUExecutionProvider alone")
        if inputs != model["inputs"]:
            raise ValueError(f"{role} ONNX input metadata mismatch")
        if outputs != model["outputs"]:
            raise ValueError(f"{role} ONNX output metadata mismatch")
        verified_by_role[role]["session"] = {
            "providers": providers,
            "inputs": inputs,
            "outputs": outputs,
        }

    recognizer_session = recognizer.session.session
    metadata = recognizer_session.get_modelmeta().custom_metadata_map
    characters = metadata.get("character")
    if characters is None:
        raise ValueError("recognizer has no embedded character vocabulary")
    vocabulary = characters.splitlines()
    required_characters = {
        character
        for image in fixture["images"]
        for region in image["regions"]
        for character in region["text_nfc"]
    }
    missing_characters = sorted(
        character for character in required_characters if character not in vocabulary
    )
    observed_vocabulary = {
        "metadata_key": "character",
        "encoding": "UTF-8 lines",
        "count": len(vocabulary),
        "sha256": hashlib.sha256(characters.encode("utf-8")).hexdigest(),
        "missing_fixture_characters": missing_characters,
    }
    expected_recognizer = candidate["recognizer"]
    if (
        observed_vocabulary["count"] != expected_recognizer["vocabulary_count"]
        or observed_vocabulary["sha256"]
        != expected_recognizer["vocabulary_sha256"]
        or missing_characters != expected_recognizer["missing_fixture_characters"]
    ):
        raise ValueError("recognizer embedded vocabulary mismatch")
    verified_by_role["recognizer"]["vocabulary"] = observed_vocabulary
    return [verified_by_role[role] for role in ("detector", "recognizer")]


def observed_polygon(box: Any, width: int, height: int) -> Polygon:
    return Polygon([(float(point[0]) / width, float(point[1]) / height) for point in box])


def expected_polygon(region: dict[str, Any]) -> Polygon:
    return Polygon([(float(point[0]), float(point[1])) for point in region["source_relative_quad"]])


def image_outcome(
    image_record: dict[str, Any],
    consumed_bytes: int,
    consumed_sha256: str,
    boxes: Any,
    texts: Any,
    scores: Any,
) -> tuple[dict[str, Any], dict[str, Any]]:
    width = image_record["width"]
    height = image_record["height"]
    observed = [
        {
            "text": normalize_text(str(text)),
            "score": float(score),
            "box": [[float(axis) for axis in point] for point in box],
        }
        for box, text, score in zip(boxes, texts, scores)
        if normalize_text(str(text))
    ]

    expected = image_record["regions"]
    exact_text_count = 0
    geometry_pass_count = 0
    confidence_pass_count = 0
    matched_observed_indexes: set[int] = set()
    matched_ids_by_index: dict[int, str] = {}
    region_results = []
    for region in expected:
        indexes = [
            index
            for index, value in enumerate(observed)
            if index not in matched_observed_indexes
            and value["text"] == region["text_nfc"]
        ]
        text_exact = len(indexes) == 1
        geometry_pass = False
        confidence_pass = False
        iou = None
        center_delta_x = None
        center_delta_y = None
        if text_exact:
            exact_text_count += 1
            index = indexes[0]
            matched_observed_indexes.add(index)
            matched_ids_by_index[index] = region["id"]
            actual = observed_polygon(observed[index]["box"], width, height)
            wanted = expected_polygon(region)
            union = actual.union(wanted).area
            iou = 0.0 if union == 0 else actual.intersection(wanted).area / union
            center_delta_x = abs(actual.centroid.x - wanted.centroid.x)
            center_delta_y = abs(actual.centroid.y - wanted.centroid.y)
            geometry_pass = (
                iou >= 0.5
                and center_delta_x <= 0.025
                and center_delta_y <= 0.025
            )
            confidence_pass = (
                math.isfinite(observed[index]["score"])
                and 0.0 <= observed[index]["score"] <= 1.0
            )
            geometry_pass_count += int(geometry_pass)
            confidence_pass_count += int(confidence_pass)
        region_results.append(
            {
                "id": region["id"],
                "text_exact": text_exact,
                "geometry_pass": geometry_pass,
                "confidence_pass": confidence_pass,
                "iou": None if iou is None else round(iou, 8),
                "center_delta_x": (
                    None if center_delta_x is None else round(center_delta_x, 8)
                ),
                "center_delta_y": (
                    None if center_delta_y is None else round(center_delta_y, 8)
                ),
                "confidence": (
                    None
                    if not text_exact
                    else round(observed[indexes[0]]["score"], 5)
                ),
            }
        )

    unmatched_indexes = [
        index
        for index in range(len(observed))
        if index not in matched_observed_indexes
    ]
    unexpected_indexes = [
        index
        for index in unmatched_indexes
        if not math.isfinite(observed[index]["score"])
        or observed[index]["score"] >= 0.5
    ]
    below_threshold_indexes = [
        index
        for index in unmatched_indexes
        if math.isfinite(observed[index]["score"])
        and observed[index]["score"] < 0.5
    ]
    admitted_indexes = sorted(
        matched_observed_indexes | set(unexpected_indexes)
    )
    expected_ids = [region["id"] for region in expected]
    admitted_order_ids = [
        matched_ids_by_index.get(index, "unexpected")
        for index in admitted_indexes
    ]
    unexpected_count = len(unexpected_indexes)
    order_pass = admitted_order_ids == expected_ids
    passed = (
        exact_text_count == len(expected)
        and unexpected_count == 0
        and geometry_pass_count == len(expected)
        and confidence_pass_count == len(expected)
        and order_pass
    )

    sanitized = {
        "file": image_record["file"],
        "fixture_sha256": image_record["sha256"],
        "consumed_fixture_sha256": consumed_sha256,
        "consumed_fixture_bytes": consumed_bytes,
        "expected_region_count": len(expected),
        "detected_region_count": len(observed),
        "admitted_region_count": len(admitted_indexes),
        "exact_text_count": exact_text_count,
        "geometry_pass_count": geometry_pass_count,
        "confidence_pass_count": confidence_pass_count,
        "unmatched_region_count": len(unmatched_indexes),
        "unexpected_region_count": unexpected_count,
        "below_unexpected_threshold_count": len(below_threshold_indexes),
        "order_pass": order_pass,
        "regions": region_results,
        "pass": passed,
    }
    raw = {
        "file": image_record["file"],
        "observed": observed,
        "admitted_order_ids": admitted_order_ids,
        "unmatched_indexes": unmatched_indexes,
        "below_unexpected_threshold_indexes": below_threshold_indexes,
        "sanitized": sanitized,
    }
    return sanitized, raw


def run_suite(
    detector: TextDetector,
    recognizer: TextRecognizer,
    fixture_inputs: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    sanitized = []
    raw = []
    for loaded in fixture_inputs:
        image_record = loaded["record"]
        image = loaded["image"]
        det_result = detector(image)
        boxes = [] if det_result.boxes is None else det_result.boxes
        crops = [
            get_rotate_crop_image(image, copy.deepcopy(box))
            for box in boxes
        ]
        if crops:
            rec_result = recognizer(
                TextRecInput(img=crops, return_word_box=False)
            )
            texts = [] if rec_result.txts is None else rec_result.txts
            scores = [] if rec_result.scores is None else rec_result.scores
        else:
            texts = []
            scores = []
        image_sanitized, image_raw = image_outcome(
            image_record,
            loaded["consumed_bytes"],
            loaded["consumed_sha256"],
            boxes,
            texts,
            scores,
        )
        sanitized.append(image_sanitized)
        raw.append(image_raw)
    return sanitized, raw


def failure_categories(images: list[dict[str, Any]]) -> list[str]:
    categories: set[str] = set()
    for image in images:
        text_complete = image["exact_text_count"] == image["expected_region_count"]
        if not text_complete:
            categories.add("text_mismatch")
        if image["admitted_region_count"] != image["expected_region_count"]:
            categories.add("region_count_mismatch")
        if image["unexpected_region_count"]:
            categories.add("unexpected_region")
        if text_complete and image["geometry_pass_count"] != image["expected_region_count"]:
            categories.add("geometry_mismatch")
        if text_complete and image["confidence_pass_count"] != image["expected_region_count"]:
            categories.add("confidence_out_of_range")
        if not image["order_pass"]:
            categories.add("ordering_mismatch")
    return sorted(categories)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--model-root", required=True, type=Path)
    parser.add_argument("--product-revision", required=True)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--raw-report", required=True, type=Path)
    args = parser.parse_args()
    report_path, raw_report_path = resolve_output_paths(
        args.report, args.raw_report
    )

    tools = verify_environment()
    host = host_record()
    if host["target"] not in {
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    }:
        raise ValueError(f"unsupported qualification target: {host['target']}")

    fixture_path = FIXTURES / "fixture-manifest.json"
    candidates_path = EVIDENCE / "candidates.json"
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    candidates = json.loads(candidates_path.read_text(encoding="utf-8"))
    fixture_inputs = load_fixture_inputs(fixture)
    candidate = next(
        (value for value in candidates["candidates"] if value["id"] == args.candidate),
        None,
    )
    if candidate is None:
        raise SystemExit(f"unknown candidate: {args.candidate}")

    model_root = args.model_root.resolve()
    verified_models = verify_models(candidate, model_root)
    detector, recognizer, initialization_ms = configure_engine(candidate, model_root)
    verified_models = verify_session_metadata(
        candidate, detector, recognizer, fixture, verified_models
    )

    for _ in range(WARMUP_PASSES):
        run_suite(detector, recognizer, fixture_inputs)

    measured_ms = []
    measured_sanitized = []
    measured_raw = []
    for _ in range(MEASURED_PASSES):
        started = time.perf_counter()
        sanitized, raw = run_suite(detector, recognizer, fixture_inputs)
        measured_ms.append((time.perf_counter() - started) * 1000.0)
        measured_sanitized.append(sanitized)
        measured_raw.append(raw)

    baseline_signature = json.dumps(measured_sanitized[0], sort_keys=True)
    stable = all(
        json.dumps(value, sort_keys=True) == baseline_signature
        for value in measured_sanitized[1:]
    )
    first = measured_sanitized[0]
    categories = failure_categories(first)
    if not stable:
        categories.append("unstable_gate_outcome")
    passed = stable and not categories and all(image["pass"] for image in first)

    report = {
        "schema_version": 2,
        "evaluator_identity": "v5",
        "fixture_profile_id": fixture["fixture_profile_id"],
        "candidate_id": candidate["id"],
        "product_base_revision": args.product_revision,
        "source_identity": {
            "evaluator_sha256": sha256(Path(__file__)),
            "fixture_manifest_sha256": sha256(fixture_path),
            "candidates_sha256": sha256(candidates_path),
            "tool_requirements_sha256": sha256(EVIDENCE / "tool-requirements.txt"),
            "rapidocr_code_sha256": tools["rapidocr_code"]["sha256"],
        },
        "host": host,
        "tools": tools,
        "models": verified_models,
        "profile": {
            "provider": "CPUExecutionProvider",
            "intra_op_threads": 1,
            "inter_op_threads": 1,
            "cpu_memory_arena": False,
            "orientation_classifier": False,
            "warmup_passes": WARMUP_PASSES,
            "measured_passes": MEASURED_PASSES,
            "unexpected_region_evaluation": "match_expected_then_threshold_unmatched",
        },
        "aggregate": {
            "initialization_ms": round(initialization_ms, 6),
            "suite_median_ms": round(statistics.median(measured_ms), 6),
            "suite_p95_ms": round(nearest_rank(measured_ms, 0.95), 6),
            "suite_maximum_ms": round(max(measured_ms), 6),
            "process_peak_resident": peak_resident_memory(),
        },
        "images": first,
        "stable_gate_outcomes": stable,
        "failure_categories": sorted(set(categories)),
        "pass": passed,
        "privacy": {
            "approved_expected_fixture_text_only": True,
            "unexpected_recognized_text_retained": False,
            "host_paths_retained": False,
            "raw_report_private_root_enforced": True,
        },
    }
    raw_report = {
        "evaluator_identity": "v5",
        "candidate_id": candidate["id"],
        "measured_ms": measured_ms,
        "passes": measured_raw,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    raw_report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    raw_report_path.write_text(
        json.dumps(raw_report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps({"candidate_id": candidate["id"], "pass": passed, "failure_categories": report["failure_categories"]}))


if __name__ == "__main__":
    main()
