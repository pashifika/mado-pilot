#!/usr/bin/env python3
"""Validate the tracked G-004 fixture and candidate evidence without network access."""

from __future__ import annotations

import hashlib
import json
import re
import struct
import unicodedata
from pathlib import Path, PurePosixPath
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = ROOT / "docs" / "evidence" / "g-004"
FIXTURES = ROOT / "fixtures" / "ocr" / "g-004"
SHA256_PATTERN = re.compile(r"[0-9a-f]{64}")
GIT_REVISION_PATTERN = re.compile(r"[0-9a-f]{40}")
V4_SOURCE_IDENTITY = {
    "evaluator_sha256": "02c78cff9bbffd7e576ab918d5d743d4a06d5b6f692a9dc0e33e0831faaeb9d4",
    "fixture_manifest_sha256": "a289edb167d45f11f4269cef22ff37d93d2cbe1150201afb9bb3f58439375c4b",
    "candidates_sha256": "033a05ed561a51994f972288a4c1594e4da52878d00e1246f0ebdbcd1d03998d",
    "tool_requirements_sha256": "7aaa23fdd2a16ed0e7607d89d070040940deb543f662ff6298a169d156a2bdc0",
}
V5_RUN_SOURCE_IDENTITY = {
    "evaluator_sha256": "780f6cccf9679bc63aeaf6829b90769032246cbcfa29746b8012865294530249",
    "fixture_manifest_sha256": "a289edb167d45f11f4269cef22ff37d93d2cbe1150201afb9bb3f58439375c4b",
    "candidates_sha256": "4b5aa66d3a7c390211219c794e35ee685701a9cd23c0f24f0d62047280199ff7",
    "tool_requirements_sha256": "7aaa23fdd2a16ed0e7607d89d070040940deb543f662ff6298a169d156a2bdc0",
    "rapidocr_code_sha256": "753f75e387f6b6d128cc644b209fb76dde04cb735de06e411d643826f0a4a5aa",
}


def fail(message: str) -> None:
    raise SystemExit(f"G-004 evidence invalid: {message}")


def load_json(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"cannot read {path.relative_to(ROOT)}: {error}")
    if not isinstance(value, dict):
        fail(f"{path.relative_to(ROOT)} must contain one JSON object")
    return value

def require_exact_keys(
    value: object,
    expected: set[str],
    field: str,
) -> dict[str, object]:
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    actual = set(value)
    if actual != expected:
        fail(
            f"{field} keys drifted: "
            f"missing={sorted(expected - actual)!r}, extra={sorted(actual - expected)!r}"
        )
    return value



def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        fail(f"cannot hash {path.relative_to(ROOT)}: {error}")
    return digest.hexdigest()

def sha256_repository_text(path: Path) -> str:
    """Hash text using the repository's canonical LF line endings."""
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for line in source:
                digest.update(line.replace(b"\r\n", b"\n"))
    except OSError as error:
        fail(f"cannot hash {path.relative_to(ROOT)}: {error}")
    return digest.hexdigest()


def require_sha256(value: object, field: str) -> str:
    if not isinstance(value, str) or SHA256_PATTERN.fullmatch(value) is None:
        fail(f"{field} must be one lowercase SHA-256 value")
    return value


def require_git_revision(value: object, field: str) -> str:
    if not isinstance(value, str) or GIT_REVISION_PATTERN.fullmatch(value) is None:
        fail(f"{field} must be one lowercase 40-hex Git revision")
    return value


def require_safe_relative_path(value: object, field: str) -> PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value:
        fail(f"{field} must be a non-empty POSIX relative path")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
        fail(f"{field} is not a safe relative path: {value!r}")
    return path


def require_https(value: object, field: str) -> str:
    if not isinstance(value, str):
        fail(f"{field} must be a URL")
    parsed = urlparse(value)
    if parsed.scheme != "https" or not parsed.netloc:
        fail(f"{field} must use an absolute HTTPS URL")
    return value


def png_dimensions(path: Path) -> tuple[int, int]:
    try:
        with path.open("rb") as source:
            header = source.read(24)
    except OSError as error:
        fail(f"cannot read {path.relative_to(ROOT)}: {error}")
    if len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n" or header[12:16] != b"IHDR":
        fail(f"{path.relative_to(ROOT)} is not a PNG with an IHDR first")
    return struct.unpack(">II", header[16:24])


def validate_fixture() -> None:
    manifest = load_json(FIXTURES / "fixture-manifest.json")
    if manifest.get("schema_version") != 1:
        fail("fixture schema_version must be 1")
    if manifest.get("fixture_profile_id") != "g-004-japanese-ui-v3":
        fail("unexpected current fixture_profile_id")
    if manifest.get("supersedes_fixture_profile_id") != "g-004-japanese-ui-v2":
        fail("current fixture must name the rejected v2 oracle")

    font = manifest.get("font")
    if not isinstance(font, dict):
        fail("fixture font record is missing")
    require_sha256(font.get("sha256"), "fixture font.sha256")
    require_https(font.get("source_url"), "fixture font.source_url")
    if font.get("license") != "OFL-1.1" or font.get("bytes_bundled") is not False:
        fail("fixture font must remain unbundled OFL-1.1 input")

    oracle = manifest.get("oracle")
    if not isinstance(oracle, dict):
        fail("fixture oracle is missing")
    if oracle.get("minimum_iou") != 0.5:
        fail("fixture minimum_iou drifted")
    if oracle.get("maximum_center_delta_x") != 0.025 or oracle.get("maximum_center_delta_y") != 0.025:
        fail("fixture center tolerance drifted")
    confidence = oracle.get("confidence")
    if (
        not isinstance(confidence, dict)
        or confidence.get("valid_range") != [0.0, 1.0]
        or confidence.get("deterministic_across_measured_passes") is not True
        or confidence.get("universal_hard_floor") is not None
    ):
        fail("fixture v2 confidence interpretation drifted")
    if oracle.get("unexpected_region_threshold") != 0.5:
        fail("fixture unexpected-region threshold drifted")

    images = manifest.get("images")
    if not isinstance(images, list) or not images:
        fail("fixture images must be a non-empty array")

    expected_sums: dict[str, str] = {}
    seen_files: set[str] = set()
    seen_region_ids: set[str] = set()
    for image_index, image in enumerate(images):
        if not isinstance(image, dict):
            fail(f"images[{image_index}] must be an object")
        relative = require_safe_relative_path(image.get("file"), f"images[{image_index}].file")
        if len(relative.parts) != 1 or relative.suffix != ".png":
            fail(f"fixture image must be one PNG filename: {relative}")
        if str(relative) in seen_files:
            fail(f"duplicate fixture image: {relative}")
        seen_files.add(str(relative))

        path = FIXTURES / relative
        digest = require_sha256(image.get("sha256"), f"images[{image_index}].sha256")
        if sha256(path) != digest:
            fail(f"fixture digest mismatch: {relative}")
        expected_sums[str(relative)] = digest

        width = image.get("width")
        height = image.get("height")
        if not isinstance(width, int) or not isinstance(height, int) or width <= 0 or height <= 0:
            fail(f"invalid dimensions for {relative}")
        if png_dimensions(path) != (width, height):
            fail(f"PNG dimensions disagree with manifest: {relative}")

        regions = image.get("regions")
        if not isinstance(regions, list) or not regions:
            fail(f"fixture regions must be non-empty: {relative}")
        if [region.get("order") for region in regions if isinstance(region, dict)] != list(range(len(regions))):
            fail(f"fixture order must be contiguous: {relative}")
        for region_index, region in enumerate(regions):
            if not isinstance(region, dict):
                fail(f"{relative} region {region_index} must be an object")
            region_id = region.get("id")
            if not isinstance(region_id, str) or not region_id or region_id in seen_region_ids:
                fail(f"invalid or duplicate region id: {region_id!r}")
            seen_region_ids.add(region_id)
            text = region.get("text_nfc")
            if not isinstance(text, str) or not text or text != unicodedata.normalize("NFC", text):
                fail(f"region {region_id} text must be non-empty NFC")
            quad = region.get("source_relative_quad")
            if not isinstance(quad, list) or len(quad) != 4:
                fail(f"region {region_id} must have four source-relative points")
            for point in quad:
                if not isinstance(point, list) or len(point) != 2:
                    fail(f"region {region_id} has an invalid point")
                if any(not isinstance(axis, (int, float)) or axis < 0 or axis > 1 for axis in point):
                    fail(f"region {region_id} point lies outside the source")

    sums_path = FIXTURES / "SHA256SUMS"
    actual_sums: dict[str, str] = {}
    for line_number, line in enumerate(sums_path.read_text(encoding="utf-8").splitlines(), start=1):
        parts = line.split("  ")
        if len(parts) != 2:
            fail(f"SHA256SUMS line {line_number} is malformed")
        digest = require_sha256(parts[0], f"SHA256SUMS line {line_number}")
        relative = require_safe_relative_path(parts[1], f"SHA256SUMS line {line_number}")
        actual_sums[str(relative)] = digest
    if actual_sums != expected_sums:
        fail("SHA256SUMS does not exactly match the fixture manifest")

    historical = load_json(FIXTURES / "fixture-manifest-v1.json")
    if sha256(FIXTURES / "fixture-manifest-v1.json") != "2d5eeaa1c19e2a15d9d395e081194a6e280df31ff5321cc827e84698f3438524":
        fail("historical v1 manifest bytes changed")
    if historical.get("schema_version") != 1 or historical.get("fixture_profile_id") != "g-004-japanese-ui-v1":
        fail("historical fixture manifest must remain v1")
    historical_images = historical.get("images")
    if not isinstance(historical_images, list) or historical_images != images[:3]:
        fail("the three v1 fixture images or expected rows changed")
    historical_v2 = load_json(FIXTURES / "fixture-manifest-v2.json")
    if sha256(FIXTURES / "fixture-manifest-v2.json") != "4d9f75a8abfb781341691252b83455e224ce440e480a1bcb9f4019f901550c22":
        fail("historical v2 manifest bytes changed")
    if (
        historical_v2.get("schema_version") != 1
        or historical_v2.get("fixture_profile_id") != "g-004-japanese-ui-v2"
        or historical_v2.get("supersedes_fixture_profile_id") != "g-004-japanese-ui-v1"
    ):
        fail("historical fixture manifest must remain v2")
    historical_v2_images = historical_v2.get("images")
    if (
        not isinstance(historical_v2_images, list)
        or len(historical_v2_images) != 5
        or historical_v2_images[:3] != images[:3]
        or historical_v2_images[4] != images[4]
    ):
        fail("v2 changed outside its rejected tooltip row")
    for historical_image in historical_v2_images:
        historical_path = FIXTURES / historical_image["file"]
        if sha256(historical_path) != historical_image["sha256"]:
            fail(f"historical v2 fixture digest mismatch: {historical_image['file']}")
    historical_oracle = historical.get("oracle")
    historical_confidence = (
        historical_oracle.get("confidence")
        if isinstance(historical_oracle, dict)
        else None
    )
    if not isinstance(historical_confidence, dict) or historical_confidence.get("minimum_per_region") != 0.8:
        fail("historical v1 confidence floor changed")


def validate_candidates() -> None:
    record = load_json(EVIDENCE / "candidates.json")
    if record.get("schema_version") != 1:
        fail("candidate schema_version must be 1")
    source = record.get("source")
    if not isinstance(source, dict):
        fail("candidate source record is missing")
    if source.get("rapidocr_version") != "3.9.2":
        fail("RapidOCR version drifted")
    require_git_revision(source.get("rapidocr_git_revision"), "rapidocr_git_revision")
    require_https(source.get("manifest"), "candidate manifest")
    require_https(source.get("model_repository"), "candidate model_repository")

    common = record.get("common_profile")
    if not isinstance(common, dict):
        fail("candidate common_profile is missing")
    expected_pixel_format = "BGR planar float32 (OpenCV order; no channel swap)"
    detector_profile = common.get("detector")
    recognizer_profile = common.get("recognizer")
    if (
        not isinstance(detector_profile, dict)
        or detector_profile.get("input_pixel_format") != expected_pixel_format
        or not isinstance(recognizer_profile, dict)
        or recognizer_profile.get("input_pixel_format") != expected_pixel_format
    ):
        fail("candidate pixel format must remain the clarified BGR profile")
    deployment = common.get("deployment")
    if not isinstance(deployment, dict):
        fail("candidate deployment is missing")
    maximum_pair_bytes = deployment.get("maximum_pair_bytes")
    if maximum_pair_bytes != 64 * 1024 * 1024:
        fail("candidate maximum_pair_bytes drifted")
    if deployment.get("ambient_search") is not False or deployment.get("network_download") is not False:
        fail("candidate deployment must prohibit ambient search and download")
    if deployment.get("verify_sha256_before_session_creation") is not True:
        fail("candidate deployment must verify SHA-256 before session creation")

    stages = record.get("evaluation_stages")
    expected_v1 = {
        "ppocrv4-japan-mobile",
        "ppocrv5-multilingual-mobile",
        "ppocrv6-multilingual-small",
    }
    expected_v2 = {
        "ppocrv4-det-v6-rec-small",
        "ppocrv5-det-v6-rec-small",
        "ppocrv6-det-tiny-rec-small",
        "ppocrv6-multilingual-small",
    }
    expected_v3 = expected_v2
    expected_v4 = expected_v3
    expected_v5 = expected_v4
    if not isinstance(stages, dict):
        fail("candidate evaluation_stages are missing")
    v1 = stages.get("v1")
    v2 = stages.get("v2")
    v3 = stages.get("v3")
    v4 = stages.get("v4")
    v5 = stages.get("v5")
    if not isinstance(v1, dict) or set(v1.get("candidates", [])) != expected_v1:
        fail("candidate v1 screening set drifted")
    if not isinstance(v2, dict) or set(v2.get("candidates", [])) != expected_v2:
        fail("candidate v2 qualification set drifted")
    if not isinstance(v3, dict) or set(v3.get("candidates", [])) != expected_v3:
        fail("candidate v3 qualification set drifted")
    if not isinstance(v4, dict) or set(v4.get("candidates", [])) != expected_v4:
        fail("candidate v4 qualification set drifted")
    expected_v5_record = {
        "frozen_at": "2026-08-22T16:53:12Z",
        "fixture_profile_id": "g-004-japanese-ui-v3",
        "fixture_manifest": "fixtures/ocr/g-004/fixture-manifest.json",
        "candidates": [
            "ppocrv4-det-v6-rec-small",
            "ppocrv5-det-v6-rec-small",
            "ppocrv6-det-tiny-rec-small",
            "ppocrv6-multilingual-small",
        ],
        "evaluator_sha256": "780f6cccf9679bc63aeaf6829b90769032246cbcfa29746b8012865294530249",
        "rapidocr_code_identity": (
            "sha256(path_length_u64be || path_utf8 || "
            "content_length_u64be || content) over .py/.yaml/.yml"
        ),
        "unexpected_region_evaluation": (
            "match_expected_then_threshold_unmatched"
        ),
        "reports": {
            "apple": (
                "docs/evidence/g-004/"
                "report-aarch64-apple-darwin-v5.json"
            ),
            "windows": (
                "docs/evidence/g-004/"
                "report-x86_64-pc-windows-msvc-v5.json"
            ),
        },
        "status": "patch-review-passed-apple-runs-authorized",
        "patch_review": {
            "status": "passed",
            "reviewed_at": "2026-08-22T17:09:59Z",
            "scope": (
                "five evaluator-integrity contracts, frozen v4 "
                "preservation, and normative G-004 v5 evidence"
            ),
        },
    }
    if v5 != expected_v5_record:
        fail("candidate v5 evaluator identity or pre-run state drifted")
    if v5["evaluator_sha256"] != sha256(EVIDENCE / "evaluate.py"):
        fail("candidate v5 evaluator source hash drifted")
    apple_v5_path = ROOT / v5["reports"]["apple"]
    windows_v5_path = ROOT / v5["reports"]["windows"]
    if not apple_v5_path.is_file():
        fail("candidate v5 Apple report is missing after authorized rerun")
    if not windows_v5_path.is_file():
        fail("candidate v5 Windows report is missing after authorized rerun")
    if v1.get("fixture_profile_id") != "g-004-japanese-ui-v1":
        fail("candidate v1 fixture profile drifted")
    if v2.get("fixture_profile_id") != "g-004-japanese-ui-v2":
        fail("candidate v2 fixture profile drifted")
    if v3.get("fixture_profile_id") != "g-004-japanese-ui-v3":
        fail("candidate v3 fixture profile drifted")
    if v4.get("fixture_profile_id") != "g-004-japanese-ui-v3":
        fail("candidate v4 fixture profile drifted")
    if v4.get("purpose") != (
        "Enforce the pinned tool environment and exact ONNX "
        "session/vocabulary identity before inference."
    ):
        fail("candidate v4 hardening purpose drifted")

    candidates = record.get("candidates")
    expected_candidates = expected_v1 | expected_v5
    if not isinstance(candidates, list) or len(candidates) != len(expected_candidates):
        fail("candidate allowlist does not match the five frozen stages")
    seen_ids: set[str] = set()
    seen_models: dict[PurePosixPath, tuple[object, object, object, object]] = {}
    for candidate_index, candidate in enumerate(candidates):
        if not isinstance(candidate, dict):
            fail(f"candidates[{candidate_index}] must be an object")
        candidate_id = candidate.get("id")
        if not isinstance(candidate_id, str) or not candidate_id or candidate_id in seen_ids:
            fail(f"invalid or duplicate candidate id: {candidate_id!r}")
        seen_ids.add(candidate_id)

        total = 0
        for role in ("detector", "recognizer"):
            model = candidate.get(role)
            if not isinstance(model, dict):
                fail(f"candidate {candidate_id} has no {role}")
            relative = require_safe_relative_path(
                model.get("relative_controlled_path"),
                f"candidate {candidate_id} {role} path",
            )
            identity = (
                model.get("id"),
                model.get("bytes"),
                model.get("sha256"),
                model.get("url"),
            )
            previous = seen_models.get(relative)
            if previous is not None and previous != identity:
                fail(f"controlled model path has conflicting identity: {relative}")
            seen_models[relative] = identity
            require_https(model.get("url"), f"candidate {candidate_id} {role} url")
            require_sha256(model.get("sha256"), f"candidate {candidate_id} {role} sha256")
            size = model.get("bytes")
            if not isinstance(size, int) or size <= 0 or size > maximum_pair_bytes:
                fail(f"candidate {candidate_id} {role} has invalid byte count")
            total += size
            inputs = model.get("inputs")
            outputs = model.get("outputs")
            if not isinstance(inputs, list) or len(inputs) != 1 or not isinstance(outputs, list) or len(outputs) != 1:
                fail(f"candidate {candidate_id} {role} must record one input and output")

        if candidate.get("total_model_bytes") != total or total > maximum_pair_bytes:
            fail(f"candidate {candidate_id} total bytes are inconsistent or over the limit")
        recognizer = candidate["recognizer"]
        count = recognizer.get("vocabulary_count")
        if not isinstance(count, int) or count <= 0:
            fail(f"candidate {candidate_id} has invalid vocabulary_count")
        require_sha256(recognizer.get("vocabulary_sha256"), f"candidate {candidate_id} vocabulary_sha256")
        if recognizer.get("missing_fixture_characters") != []:
            fail(f"candidate {candidate_id} lacks fixture characters")
        output_shape = recognizer["outputs"][0].get("shape")
        if not isinstance(output_shape, list) or output_shape[-1] != count + 2:
            fail(f"candidate {candidate_id} output classes must equal vocabulary plus blank and space")
    if seen_ids != expected_candidates:
        fail("candidate records do not exactly cover the frozen stages")


def validate_apple_report() -> None:
    report = load_json(EVIDENCE / "report-aarch64-apple-darwin.json")
    require_exact_keys(
        report,
        {
            "schema_version",
            "report_kind",
            "qualification_stage",
            "target",
            "product_base_revision",
            "metadata_clarification",
            "host",
            "tools",
            "profile",
            "stages",
            "apple_decision",
            "privacy",
        },
        "Apple report",
    )
    if report.get("report_kind") != "g-004-cross-candidate-qualification":
        fail("Apple report kind drifted")
    if report.get("schema_version") != 2:
        fail("Apple report schema_version must be 2")
    if report.get("qualification_stage") != "v4":
        fail("Apple report must use the hardened v4 evaluator")
    if report.get("target") != "aarch64-apple-darwin":
        fail("Apple report target must be aarch64-apple-darwin")
    if report.get("product_base_revision") != "f3608424dde88f835f35653be8113f7a2009431b":
        fail("Apple report product baseline drifted")
    if report.get("host") != {
        "target": "aarch64-apple-darwin",
        "os": "Darwin",
        "os_release": "25.5.0",
        "os_version": "Darwin Kernel Version 25.5.0: Tue Jun  9 22:18:58 PDT 2026; root:xnu-12377.121.10~1/RELEASE_ARM64_T6000",
        "architecture": "arm64",
        "logical_cpu_count": 10,
        "physical_memory_bytes": 34359738368,
    }:
        fail("Apple host identity is invalid")

    stages = report.get("stages")
    if (
        not isinstance(stages, list)
        or [stage.get("id") for stage in stages if isinstance(stage, dict)]
        != ["v1", "v2", "v3", "v4"]
    ):
        fail("Apple report must retain v1 through v4 in order")

    def frozen_candidates_sha256(candidates: object) -> str:
        payload = json.dumps(
            candidates,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        return hashlib.sha256(payload).hexdigest()

    expected_stage_hashes = {
        "v1": "271826b8da10c295a1e5e04717fa82f08d5979d98c84fb9895aaaaa51fb1643e",
        "v2": "50e8bd23b9f411c1fc6ba3d1910c6dfd1b5e59fadadbcf1879945169db2c0a85",
        "v3": "ad11cbf9fe332f7fc2472b3a00c7b172c902a02faef21b95bbe00255ce3d43d6",
        "v4": "63ae5f33d0a3f76c659f68f4cfe4e393dbf269eded9ce3005b2cb07ac50e428f",
    }
    expected_stage_statuses = {
        "v1": "rejected-historical",
        "v2": "rejected-historical",
        "v3": "superseded-evaluator-unenforced-identity",
        "v4": "apple-complete-windows-pending",
    }
    expected_stage_reasons = {
        "v1": "Two recognizers failed exact text; the universal confidence floor was disproved before v2.",
        "v2": "The first amended candidate exposed a tooltip manifest-order authoring defect; all non-order rows passed.",
        "v3": "The fixture results are retained, but the evaluator did not enforce every pinned package version or loaded ONNX/vocabulary field.",
        "v4": "The evaluator enforced the complete pinned environment and exact CPU session/input/output/vocabulary identity before inference.",
    }
    for stage in stages:
        require_exact_keys(
            stage,
            {"id", "status", "reason", "candidates", "frozen_candidates_sha256"},
            f"Apple stage {stage.get('id')}",
        )
        stage_id = stage["id"]
        candidates = stage.get("candidates")
        if not isinstance(candidates, list) or not candidates:
            fail(f"Apple stage {stage_id} has no candidates")
        observed_hash = frozen_candidates_sha256(candidates)
        if (
            stage.get("frozen_candidates_sha256") != observed_hash
            or observed_hash != expected_stage_hashes[stage_id]
        ):
            fail(f"Apple frozen {stage_id} candidate outcomes changed")
        if (
            stage.get("status") != expected_stage_statuses[stage_id]
            or stage.get("reason") != expected_stage_reasons[stage_id]
        ):
            fail(f"Apple stage {stage_id} metadata changed")

    historical_identities = {
        "v1": {
            "evaluator_sha256": "befa8fe0481e82b10d4dd36689ebf61af1b70c529fbdc1f631226dd74894a1bf",
            "fixture_manifest_sha256": "2d5eeaa1c19e2a15d9d395e081194a6e280df31ff5321cc827e84698f3438524",
            "candidates_sha256": "64eb2cfc255b00ef044a88a3cf37fe2558a6bf16036c22ede536f20a5f1d3d46",
            "tool_requirements_sha256": "7aaa23fdd2a16ed0e7607d89d070040940deb543f662ff6298a169d156a2bdc0",
        },
        "v2": {
            "evaluator_sha256": "c81aec256ebec02aa9fdd6a35bf287021bb627755b8f3b5a45bfcf0a4627d8dc",
            "fixture_manifest_sha256": "4d9f75a8abfb781341691252b83455e224ce440e480a1bcb9f4019f901550c22",
            "candidates_sha256": "213323b7d70cea8448c4799742283072ad32c7330304065df835d1b7886c5f66",
            "tool_requirements_sha256": "7aaa23fdd2a16ed0e7607d89d070040940deb543f662ff6298a169d156a2bdc0",
        },
        "v3": {
            "evaluator_sha256": "77f808eac65f5b09157569b1c89b48c74501986f13133e6201559be512628165",
            "fixture_manifest_sha256": "a289edb167d45f11f4269cef22ff37d93d2cbe1150201afb9bb3f58439375c4b",
            "candidates_sha256": "c77239560b4f93930b19b30cb708c6736151fef3eb9a6fd0bc846e0ab28aa85b",
            "tool_requirements_sha256": "7aaa23fdd2a16ed0e7607d89d070040940deb543f662ff6298a169d156a2bdc0",
        },
    }
    for stage in stages[:3]:
        expected_identity = historical_identities[stage["id"]]
        for candidate in stage["candidates"]:
            if candidate.get("source_identity") != expected_identity:
                fail(f"Apple historical {stage['id']} source identity changed")

    current_identity = V4_SOURCE_IDENTITY
    candidate_record = load_json(EVIDENCE / "candidates.json")
    candidate_lookup = {
        candidate["id"]: candidate for candidate in candidate_record["candidates"]
    }
    fixture = load_json(FIXTURES / "fixture-manifest.json")
    fixture_lookup = {image["file"]: image for image in fixture["images"]}
    expected_v4 = {
        "ppocrv4-det-v6-rec-small",
        "ppocrv5-det-v6-rec-small",
        "ppocrv6-det-tiny-rec-small",
        "ppocrv6-multilingual-small",
    }
    selected = "ppocrv4-det-v6-rec-small"
    v4_candidates = stages[3]["candidates"]
    if {candidate.get("candidate_id") for candidate in v4_candidates} != expected_v4:
        fail("Apple v4 candidate matrix drifted")

    requirements = {}
    expected_python = None
    for line in (EVIDENCE / "tool-requirements.txt").read_text(encoding="utf-8").splitlines():
        if line.startswith("# Python "):
            expected_python = line.removeprefix("# Python ").split(";", 1)[0]
        elif line and not line.startswith("#"):
            name, version = line.split("==", 1)
            requirements[name] = version
    expected_tools = {
        "python": expected_python,
        "packages": requirements,
        "modules": {
            "onnxruntime": "1.29.0",
            "opencv": "5.0.0",
            "pillow": "12.3.0",
        },
        "onnxruntime_available_providers": [
            "CoreMLExecutionProvider",
            "AzureExecutionProvider",
            "CPUExecutionProvider",
        ],
    }
    if report.get("tools") != expected_tools:
        fail("Apple v4 tool environment drifted")
    if report.get("profile") != {
        "provider": "CPUExecutionProvider",
        "intra_op_threads": 1,
        "inter_op_threads": 1,
        "cpu_memory_arena": False,
        "orientation_classifier": False,
        "warmup_passes": 2,
        "measured_passes": 10,
    }:
        fail("Apple v4 execution profile drifted")

    expected_vocabulary = {
        "metadata_key": "character",
        "encoding": "UTF-8 lines",
        "count": 18708,
        "sha256": "f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e",
        "missing_fixture_characters": [],
    }
    for candidate in v4_candidates:
        candidate_id = candidate["candidate_id"]
        if candidate.get("fixture_profile_id") != "g-004-japanese-ui-v3":
            fail(f"Apple v4 candidate {candidate_id} uses the wrong fixture")
        if candidate.get("source_identity") != current_identity:
            fail(f"Apple v4 candidate {candidate_id} source identity is stale")
        if candidate.get("stable_gate_outcomes") is not True:
            fail(f"Apple v4 candidate {candidate_id} is unstable")

        expected_candidate = candidate_lookup[candidate_id]
        expected_models = []
        for role in ("detector", "recognizer"):
            model = expected_candidate[role]
            expected_model = {
                "role": role,
                "id": model["id"],
                "relative_controlled_path": model["relative_controlled_path"],
                "bytes": model["bytes"],
                "sha256": model["sha256"],
                "session": {
                    "providers": ["CPUExecutionProvider"],
                    "inputs": model["inputs"],
                    "outputs": model["outputs"],
                },
            }
            if role == "recognizer":
                expected_model["vocabulary"] = expected_vocabulary
            expected_models.append(expected_model)
        if candidate.get("models") != expected_models:
            fail(f"Apple v4 candidate {candidate_id} loaded model identity drifted")

        images = candidate.get("images")
        if not isinstance(images, list) or [image.get("file") for image in images] != list(fixture_lookup):
            fail(f"Apple v4 candidate {candidate_id} image matrix drifted")
        for image in images:
            expected_image = fixture_lookup[image["file"]]
            if image.get("fixture_sha256") != expected_image["sha256"]:
                fail(f"Apple v4 candidate {candidate_id} fixture digest drifted")
            if image.get("expected_region_count") != len(expected_image["regions"]):
                fail(f"Apple v4 candidate {candidate_id} expected count drifted")

        if candidate_id == selected:
            if candidate.get("pass") is not True or candidate.get("failure_categories") != []:
                fail("Apple selected v4 candidate no longer passes")
            for image in images:
                expected_count = image["expected_region_count"]
                if (
                    image.get("observed_region_count") != expected_count
                    or image.get("exact_text_count") != expected_count
                    or image.get("geometry_pass_count") != expected_count
                    or image.get("confidence_pass_count") != expected_count
                    or image.get("unexpected_region_count") != 0
                    or image.get("order_pass") is not True
                    or image.get("pass") is not True
                ):
                    fail(f"Apple selected v4 candidate fails {image['file']}")
        elif (
            candidate.get("pass") is not False
            or candidate.get("failure_categories")
            != ["ordering_mismatch", "text_mismatch", "unexpected_region"]
        ):
            fail(f"Apple rejected v4 candidate {candidate_id} outcome drifted")

    clarification = report.get("metadata_clarification")
    if clarification != {
        "fields": [
            "common_profile.detector.input_pixel_format",
            "common_profile.recognizer.input_pixel_format",
        ],
        "candidate_manifest_sha256_at_v3_run": "c77239560b4f93930b19b30cb708c6736151fef3eb9a6fd0bc846e0ab28aa85b",
        "first_corrected_candidate_manifest_sha256": "22ed31eae6f7bd873790b12f815f25465af60ff104c6bd6ddf6396015c1ff2a0",
        "current_candidate_manifest_sha256": current_identity["candidates_sha256"],
        "at_run_label": "RGB planar float32",
        "executed_and_current_value": "BGR planar float32 (OpenCV order; no channel swap)",
        "execution_changed": False,
        "measurements_replaced_by_hardened_v4": True,
    }:
        fail("Apple report pixel-format clarification drifted")

    decision = report.get("apple_decision")
    if decision != {
        "selected_candidate_for_windows": selected,
        "reason": "Only this candidate passed every v4 environment, model/session/vocabulary, text, count, geometry, ordering, confidence-validity, stability, digest, license, and deployment row.",
        "other_v4_candidates_rejected": [
            "ppocrv5-det-v6-rec-small",
            "ppocrv6-det-tiny-rec-small",
            "ppocrv6-multilingual-small",
        ],
        "g004_status": "open-pending-windows",
    }:
        fail("Apple decision record drifted")

    if report.get("privacy") != {
        "approved_expected_fixture_text_only": True,
        "unexpected_recognized_text_retained": False,
        "game_screenshot_pixels_or_text_retained": False,
        "host_paths_retained": False,
    }:
        fail("Apple report privacy declaration drifted")

    def reject_unapproved_payload(value: object) -> None:
        if isinstance(value, dict):
            if any(key in {"text", "observed", "raw"} for key in value):
                fail("Apple report contains unapproved recognized payload fields")
            for nested in value.values():
                reject_unapproved_payload(nested)
        elif isinstance(value, list):
            for nested in value:
                reject_unapproved_payload(nested)
        elif isinstance(value, str) and ("/Users/" in value or "\\Users\\" in value):
            fail("Apple report contains a host-local path")

    reject_unapproved_payload(report)

def validate_windows_report() -> None:
    report = load_json(EVIDENCE / "report-x86_64-pc-windows-msvc.json")
    require_exact_keys(
        report,
        {
            "schema_version",
            "report_kind",
            "qualification_stage",
            "target",
            "product_base_revision",
            "host",
            "tools",
            "profile",
            "candidates",
            "frozen_candidates_sha256",
            "cross_target_decision",
            "privacy",
        },
        "Windows report",
    )
    if report.get("schema_version") != 1:
        fail("Windows report schema_version must be 1")
    if report.get("report_kind") != "g-004-cross-candidate-qualification":
        fail("Windows report kind drifted")
    if report.get("qualification_stage") != "v4":
        fail("Windows report must use the hardened v4 evaluator")
    if report.get("target") != "x86_64-pc-windows-msvc":
        fail("Windows report target must be x86_64-pc-windows-msvc")
    if report.get("product_base_revision") != "f3608424dde88f835f35653be8113f7a2009431b":
        fail("Windows report product baseline drifted")

    if report.get("host") != {
        "target": "x86_64-pc-windows-msvc",
        "os": "Windows",
        "os_release": "11",
        "os_version": "10.0.26200",
        "architecture": "AMD64",
        "logical_cpu_count": 20,
        "physical_memory_bytes": 34197635072,
    }:
        fail("Windows host identity is invalid")

    requirements: dict[str, str] = {}
    expected_python = None
    for line in (EVIDENCE / "tool-requirements.txt").read_text(encoding="utf-8").splitlines():
        if line.startswith("# Python "):
            expected_python = line.removeprefix("# Python ").split(";", 1)[0]
        elif line and not line.startswith("#"):
            name, version = line.split("==", 1)
            requirements[name] = version
    expected_tools = {
        "python": expected_python,
        "packages": requirements,
        "modules": {
            "onnxruntime": "1.29.0",
            "opencv": "5.0.0",
            "pillow": "12.3.0",
        },
        "onnxruntime_available_providers": [
            "AzureExecutionProvider",
            "CPUExecutionProvider",
        ],
    }
    if report.get("tools") != expected_tools:
        fail("Windows v4 tool environment drifted")
    if report.get("profile") != {
        "provider": "CPUExecutionProvider",
        "intra_op_threads": 1,
        "inter_op_threads": 1,
        "cpu_memory_arena": False,
        "orientation_classifier": False,
        "warmup_passes": 2,
        "measured_passes": 10,
    }:
        fail("Windows v4 execution profile drifted")

    candidate_record = load_json(EVIDENCE / "candidates.json")
    candidate_lookup = {
        candidate["id"]: candidate for candidate in candidate_record["candidates"]
    }
    fixture = load_json(FIXTURES / "fixture-manifest.json")
    fixture_lookup = {image["file"]: image for image in fixture["images"]}
    current_identity = V4_SOURCE_IDENTITY
    expected_candidate_ids = [
        "ppocrv4-det-v6-rec-small",
        "ppocrv5-det-v6-rec-small",
        "ppocrv6-det-tiny-rec-small",
        "ppocrv6-multilingual-small",
    ]
    selected = expected_candidate_ids[0]
    candidates = report.get("candidates")
    if (
        not isinstance(candidates, list)
        or [candidate.get("candidate_id") for candidate in candidates if isinstance(candidate, dict)]
        != expected_candidate_ids
    ):
        fail("Windows v4 candidate matrix drifted")
    frozen_payload = json.dumps(
        candidates,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    observed_candidates_sha256 = hashlib.sha256(frozen_payload).hexdigest()
    if (
        report.get("frozen_candidates_sha256") != observed_candidates_sha256
        or observed_candidates_sha256
        != "c6e6a5dab741ef7ad3bda5587cf4ce8138570da92dc2262d3823ca5b5bdf88da"
    ):
        fail("Windows frozen v4 candidate outcomes changed")

    expected_vocabulary = {
        "metadata_key": "character",
        "encoding": "UTF-8 lines",
        "count": 18708,
        "sha256": "f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e",
        "missing_fixture_characters": [],
    }
    for candidate in candidates:
        candidate_id = candidate["candidate_id"]
        if candidate.get("fixture_profile_id") != "g-004-japanese-ui-v3":
            fail(f"Windows v4 candidate {candidate_id} uses the wrong fixture")
        if candidate.get("source_identity") != current_identity:
            fail(f"Windows v4 candidate {candidate_id} source identity is stale")
        if candidate.get("stable_gate_outcomes") is not True:
            fail(f"Windows v4 candidate {candidate_id} is unstable")

        expected_candidate = candidate_lookup[candidate_id]
        expected_models = []
        for role in ("detector", "recognizer"):
            model = expected_candidate[role]
            expected_model = {
                "role": role,
                "id": model["id"],
                "relative_controlled_path": model["relative_controlled_path"],
                "bytes": model["bytes"],
                "sha256": model["sha256"],
                "session": {
                    "providers": ["CPUExecutionProvider"],
                    "inputs": model["inputs"],
                    "outputs": model["outputs"],
                },
            }
            if role == "recognizer":
                expected_model["vocabulary"] = expected_vocabulary
            expected_models.append(expected_model)
        if candidate.get("models") != expected_models:
            fail(f"Windows v4 candidate {candidate_id} loaded model identity drifted")

        aggregate = candidate.get("aggregate")
        if not isinstance(aggregate, dict):
            fail(f"Windows v4 candidate {candidate_id} aggregate is missing")
        initialization = aggregate.get("initialization_ms")
        median = aggregate.get("suite_median_ms")
        p95 = aggregate.get("suite_p95_ms")
        maximum = aggregate.get("suite_maximum_ms")
        if (
            not all(isinstance(value, (int, float)) and value > 0 for value in (
                initialization,
                median,
                p95,
                maximum,
            ))
            or not median <= p95 <= maximum
            or aggregate.get("process_peak_resident_bytes") is not None
        ):
            fail(f"Windows v4 candidate {candidate_id} aggregate is invalid")

        images = candidate.get("images")
        if not isinstance(images, list) or [image.get("file") for image in images] != list(fixture_lookup):
            fail(f"Windows v4 candidate {candidate_id} image matrix drifted")
        for image in images:
            expected_image = fixture_lookup[image["file"]]
            if image.get("fixture_sha256") != expected_image["sha256"]:
                fail(f"Windows v4 candidate {candidate_id} fixture digest drifted")
            if image.get("expected_region_count") != len(expected_image["regions"]):
                fail(f"Windows v4 candidate {candidate_id} expected count drifted")

        if candidate_id == selected:
            if candidate.get("pass") is not True or candidate.get("failure_categories") != []:
                fail("Windows selected v4 candidate no longer passes")
            for image in images:
                expected_count = image["expected_region_count"]
                if (
                    image.get("observed_region_count") != expected_count
                    or image.get("exact_text_count") != expected_count
                    or image.get("geometry_pass_count") != expected_count
                    or image.get("confidence_pass_count") != expected_count
                    or image.get("unexpected_region_count") != 0
                    or image.get("order_pass") is not True
                    or image.get("pass") is not True
                ):
                    fail(f"Windows selected v4 candidate fails {image['file']}")
        elif (
            candidate.get("pass") is not False
            or candidate.get("failure_categories")
            != ["ordering_mismatch", "text_mismatch", "unexpected_region"]
        ):
            fail(f"Windows rejected v4 candidate {candidate_id} outcome drifted")

    apple_path = EVIDENCE / "report-aarch64-apple-darwin.json"
    expected_decision = {
        "apple_report_sha256": sha256_repository_text(apple_path),
        "conditional_profile_id": "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1",
        "conditional_candidate": selected,
        "candidate_gate_outcomes_match_apple": True,
        "image_gate_outcomes_match_apple": True,
        "other_v4_candidates_rejected": expected_candidate_ids[1:],
        "unresolved_evidence_gaps": [
            "fixture_bytes_not_bound_at_evaluation_time",
            "rapidocr_code_bytes_not_bound_to_declared_revision",
            "unexpected_region_threshold_applied_before_expected_matching",
            "raw_report_path_not_constrained_to_ignored_ephemera",
            "windows_peak_resident_failure_reason_not_recorded",
        ],
        "g004_status": "open-independent-review-findings",
    }
    if report.get("cross_target_decision") != expected_decision:
        fail("Windows cross-target decision drifted")

    apple_report = load_json(apple_path)
    apple_candidates = apple_report["stages"][3]["candidates"]
    apple_lookup = {
        candidate["candidate_id"]: candidate for candidate in apple_candidates
    }
    candidate_gate_fields = ("stable_gate_outcomes", "failure_categories", "pass")
    image_gate_fields = (
        "file",
        "fixture_sha256",
        "expected_region_count",
        "observed_region_count",
        "exact_text_count",
        "geometry_pass_count",
        "confidence_pass_count",
        "unexpected_region_count",
        "order_pass",
        "pass",
    )
    for candidate in candidates:
        candidate_id = candidate["candidate_id"]
        apple_candidate = apple_lookup.get(candidate_id)
        if apple_candidate is None:
            fail(f"Apple v4 candidate {candidate_id} is missing")
        if any(candidate.get(field) != apple_candidate.get(field) for field in candidate_gate_fields):
            fail(f"candidate gate outcome diverged for {candidate_id}")
        apple_images = apple_candidate.get("images")
        if not isinstance(apple_images, list) or len(candidate["images"]) != len(apple_images):
            fail(f"image matrix diverged for {candidate_id}")
        for windows_image, apple_image in zip(candidate["images"], apple_images, strict=True):
            if any(windows_image.get(field) != apple_image.get(field) for field in image_gate_fields):
                fail(f"image gate outcome diverged for {candidate_id}/{windows_image['file']}")

    if report.get("privacy") != {
        "approved_expected_fixture_text_only": True,
        "unexpected_recognized_text_retained": False,
        "game_screenshot_pixels_or_text_retained": False,
        "host_paths_retained": False,
    }:
        fail("Windows report privacy declaration drifted")

    def reject_unapproved_payload(value: object) -> None:
        if isinstance(value, dict):
            if any(key in {"text", "observed", "raw"} for key in value):
                fail("Windows report contains unapproved recognized payload fields")
            for nested in value.values():
                reject_unapproved_payload(nested)
        elif isinstance(value, list):
            for nested in value:
                reject_unapproved_payload(nested)
        elif isinstance(value, str) and ("/Users/" in value or "\\Users\\" in value):
            fail("Windows report contains a host-local path")

    reject_unapproved_payload(report)



def validate_v5_tracked_source_identity() -> None:
    tracked_inputs = {
        "evaluator_sha256": EVIDENCE / "evaluate.py",
        "fixture_manifest_sha256": FIXTURES / "fixture-manifest.json",
        "candidates_sha256": EVIDENCE / "candidates.json",
        "tool_requirements_sha256": EVIDENCE / "tool-requirements.txt",
    }
    for field, path in tracked_inputs.items():
        if sha256(path) != V5_RUN_SOURCE_IDENTITY[field]:
            fail(f"v5 {field} no longer matches the tracked input bytes")


def validate_apple_v5_report() -> None:
    path = EVIDENCE / "report-aarch64-apple-darwin-v5.json"
    report = load_json(path)
    require_exact_keys(
        report,
        {
            "schema_version",
            "report_kind",
            "qualification_stage",
            "target",
            "product_base_revision",
            "source_identity",
            "host",
            "tools",
            "profile",
            "patch_review",
            "historical_v4_report",
            "candidates",
            "candidate_outcomes_sha256",
            "apple_decision",
            "privacy",
        },
        "Apple v5 report",
    )
    if report.get("schema_version") != 1:
        fail("Apple v5 report schema_version must be 1")
    if report.get("report_kind") != "g-004-evaluator-v5-target-qualification":
        fail("Apple v5 report kind drifted")
    if report.get("qualification_stage") != "v5":
        fail("Apple v5 qualification stage drifted")
    if report.get("target") != "aarch64-apple-darwin":
        fail("Apple v5 target drifted")
    if report.get("product_base_revision") != "f3608424dde88f835f35653be8113f7a2009431b":
        fail("Apple v5 product baseline drifted")
    if report.get("source_identity") != V5_RUN_SOURCE_IDENTITY:
        fail("Apple v5 source identity drifted")
    if sha256(EVIDENCE / "evaluate.py") != V5_RUN_SOURCE_IDENTITY["evaluator_sha256"]:
        fail("Apple v5 evaluator no longer matches its run identity")

    requirements = {}
    expected_python = None
    for line in (EVIDENCE / "tool-requirements.txt").read_text(encoding="utf-8").splitlines():
        if line.startswith("# Python "):
            expected_python = line.removeprefix("# Python ").split(";", 1)[0]
        elif line and not line.startswith("#"):
            name, version = line.split("==", 1)
            requirements[name] = version
    expected_rapidocr_code = {
        "algorithm": (
            "sha256(path_length_u64be || path_utf8 || "
            "content_length_u64be || content)"
        ),
        "included_suffixes": [".py", ".yaml", ".yml"],
        "file_count": 86,
        "total_bytes": 625123,
        "sha256": V5_RUN_SOURCE_IDENTITY["rapidocr_code_sha256"],
    }
    if report.get("tools") != {
        "python": expected_python,
        "packages": requirements,
        "modules": {
            "onnxruntime": "1.29.0",
            "opencv": "5.0.0",
            "pillow": "12.3.0",
        },
        "onnxruntime_available_providers": [
            "CoreMLExecutionProvider",
            "AzureExecutionProvider",
            "CPUExecutionProvider",
        ],
        "rapidocr_code": expected_rapidocr_code,
    }:
        fail("Apple v5 tool or RapidOCR code identity drifted")
    if report.get("profile") != {
        "provider": "CPUExecutionProvider",
        "intra_op_threads": 1,
        "inter_op_threads": 1,
        "cpu_memory_arena": False,
        "orientation_classifier": False,
        "warmup_passes": 2,
        "measured_passes": 10,
        "unexpected_region_evaluation": (
            "match_expected_then_threshold_unmatched"
        ),
    }:
        fail("Apple v5 execution profile drifted")
    if report.get("patch_review") != {
        "status": "passed-before-runs",
        "reviewed_at": "2026-08-22T17:09:59Z",
        "scope": (
            "five evaluator-integrity contracts, frozen v4 preservation, "
            "and normative G-004 v5 evidence"
        ),
    }:
        fail("Apple v5 patch-review record drifted")

    v4_path = EVIDENCE / "report-aarch64-apple-darwin.json"
    if report.get("historical_v4_report") != {
        "path": "docs/evidence/g-004/report-aarch64-apple-darwin.json",
        "sha256": sha256_repository_text(v4_path),
        "status": "immutable-audit-record-not-qualification",
    }:
        fail("Apple v5 historical v4 binding drifted")

    candidates = report.get("candidates")
    expected_candidate_ids = [
        "ppocrv4-det-v6-rec-small",
        "ppocrv5-det-v6-rec-small",
        "ppocrv6-det-tiny-rec-small",
        "ppocrv6-multilingual-small",
    ]
    if (
        not isinstance(candidates, list)
        or [candidate.get("candidate_id") for candidate in candidates]
        != expected_candidate_ids
    ):
        fail("Apple v5 candidate matrix drifted")
    candidates_payload = json.dumps(
        candidates,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    observed_outcomes_sha256 = hashlib.sha256(candidates_payload).hexdigest()
    if (
        report.get("candidate_outcomes_sha256") != observed_outcomes_sha256
        or observed_outcomes_sha256
        != "5c5eb4849c65b0047ab11f0cd8ab33033594e29d520e05aef7fb0c0cd14d1730"
    ):
        fail("Apple v5 frozen candidate outcomes changed")

    candidate_record = load_json(EVIDENCE / "candidates.json")
    candidate_lookup = {
        candidate["id"]: candidate for candidate in candidate_record["candidates"]
    }
    fixture = load_json(FIXTURES / "fixture-manifest.json")
    fixture_lookup = {image["file"]: image for image in fixture["images"]}
    expected_vocabulary = {
        "metadata_key": "character",
        "encoding": "UTF-8 lines",
        "count": 18708,
        "sha256": "f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e",
        "missing_fixture_characters": [],
    }
    selected = expected_candidate_ids[0]
    for candidate in candidates:
        candidate_id = candidate["candidate_id"]
        require_exact_keys(
            candidate,
            {
                "candidate_id",
                "source_identity",
                "models",
                "aggregate",
                "images",
                "stable_gate_outcomes",
                "failure_categories",
                "pass",
            },
            f"Apple v5 candidate {candidate_id}",
        )
        if candidate["source_identity"] != V5_RUN_SOURCE_IDENTITY:
            fail(f"Apple v5 candidate {candidate_id} source identity drifted")
        if candidate["stable_gate_outcomes"] is not True:
            fail(f"Apple v5 candidate {candidate_id} is unstable")

        expected_candidate = candidate_lookup[candidate_id]
        expected_models = []
        for role in ("detector", "recognizer"):
            model = expected_candidate[role]
            expected_model = {
                "role": role,
                "id": model["id"],
                "relative_controlled_path": model["relative_controlled_path"],
                "bytes": model["bytes"],
                "sha256": model["sha256"],
                "session": {
                    "providers": ["CPUExecutionProvider"],
                    "inputs": model["inputs"],
                    "outputs": model["outputs"],
                },
            }
            if role == "recognizer":
                expected_model["vocabulary"] = expected_vocabulary
            expected_models.append(expected_model)
        if candidate["models"] != expected_models:
            fail(f"Apple v5 candidate {candidate_id} model identity drifted")

        aggregate = require_exact_keys(
            candidate["aggregate"],
            {
                "initialization_ms",
                "suite_median_ms",
                "suite_p95_ms",
                "suite_maximum_ms",
                "process_peak_resident",
            },
            f"Apple v5 candidate {candidate_id} aggregate",
        )
        resident = aggregate["process_peak_resident"]
        if (
            not isinstance(resident, dict)
            or resident.get("status") != "measured"
            or not isinstance(resident.get("bytes"), int)
            or resident["bytes"] <= 0
            or resident.get("source") != "getrusage.ru_maxrss"
        ):
            fail(f"Apple v5 candidate {candidate_id} resident outcome drifted")

        images = candidate["images"]
        if [image.get("file") for image in images] != list(fixture_lookup):
            fail(f"Apple v5 candidate {candidate_id} image matrix drifted")
        for image in images:
            require_exact_keys(
                image,
                {
                    "file",
                    "fixture_sha256",
                    "consumed_fixture_sha256",
                    "consumed_fixture_bytes",
                    "expected_region_count",
                    "detected_region_count",
                    "admitted_region_count",
                    "exact_text_count",
                    "geometry_pass_count",
                    "confidence_pass_count",
                    "unmatched_region_count",
                    "unexpected_region_count",
                    "below_unexpected_threshold_count",
                    "order_pass",
                    "minimum_matched_iou",
                    "minimum_matched_confidence",
                    "pass",
                },
                f"Apple v5 candidate {candidate_id} image",
            )
            expected_image = fixture_lookup[image["file"]]
            fixture_path = FIXTURES / image["file"]
            if (
                image["fixture_sha256"] != expected_image["sha256"]
                or image["consumed_fixture_sha256"] != expected_image["sha256"]
                or image["consumed_fixture_bytes"] != fixture_path.stat().st_size
            ):
                fail(f"Apple v5 candidate {candidate_id} consumed fixture drifted")

        if candidate_id == selected:
            if candidate["pass"] is not True or candidate["failure_categories"] != []:
                fail("Apple v5 selected candidate no longer passes")
            for image in images:
                expected_count = image["expected_region_count"]
                if (
                    image["detected_region_count"] != expected_count
                    or image["admitted_region_count"] != expected_count
                    or image["exact_text_count"] != expected_count
                    or image["geometry_pass_count"] != expected_count
                    or image["confidence_pass_count"] != expected_count
                    or image["unmatched_region_count"] != 0
                    or image["unexpected_region_count"] != 0
                    or image["below_unexpected_threshold_count"] != 0
                    or image["order_pass"] is not True
                    or image["pass"] is not True
                ):
                    fail(f"Apple v5 selected candidate fails {image['file']}")
        elif (
            candidate["pass"] is not False
            or candidate["failure_categories"]
            != ["ordering_mismatch", "text_mismatch", "unexpected_region"]
        ):
            fail(f"Apple v5 rejected candidate {candidate_id} outcome drifted")

    if report.get("apple_decision") != {
        "selected_candidate_for_windows_v5": selected,
        "other_candidates_rejected": expected_candidate_ids[1:],
        "g004_status": "open-pending-windows-v5",
    }:
        fail("Apple v5 decision drifted")
    if report.get("privacy") != {
        "approved_expected_fixture_text_only": True,
        "unexpected_recognized_text_retained": False,
        "game_screenshot_pixels_or_text_retained": False,
        "host_paths_retained": False,
        "raw_report_private_root_enforced": True,
    }:
        fail("Apple v5 privacy declaration drifted")

    def reject_unapproved_payload(value: object) -> None:
        if isinstance(value, dict):
            if any(key in {"text", "observed", "raw"} for key in value):
                fail("Apple v5 report contains unapproved recognized payload fields")
            for nested in value.values():
                reject_unapproved_payload(nested)
        elif isinstance(value, list):
            for nested in value:
                reject_unapproved_payload(nested)
        elif isinstance(value, str) and ("/Users/" in value or "\\Users\\" in value):
            fail("Apple v5 report contains a host-local path")

    reject_unapproved_payload(report)


def validate_windows_v5_report() -> None:
    path = EVIDENCE / "report-x86_64-pc-windows-msvc-v5.json"
    report = load_json(path)
    require_exact_keys(
        report,
        {
            "schema_version",
            "report_kind",
            "qualification_stage",
            "target",
            "product_base_revision",
            "source_identity",
            "host",
            "tools",
            "profile",
            "patch_review",
            "final_evidence_review",
            "historical_v4_report",
            "apple_v5_report",
            "candidates",
            "candidate_outcomes_sha256",
            "cross_target_decision",
            "privacy",
        },
        "Windows v5 report",
    )
    if report.get("schema_version") != 1:
        fail("Windows v5 report schema_version must be 1")
    if report.get("report_kind") != "g-004-evaluator-v5-cross-target-qualification":
        fail("Windows v5 report kind drifted")
    if report.get("qualification_stage") != "v5":
        fail("Windows v5 qualification stage drifted")
    if report.get("target") != "x86_64-pc-windows-msvc":
        fail("Windows v5 target drifted")
    if report.get("product_base_revision") != "f3608424dde88f835f35653be8113f7a2009431b":
        fail("Windows v5 product baseline drifted")
    if report.get("source_identity") != V5_RUN_SOURCE_IDENTITY:
        fail("Windows v5 source identity drifted")
    if report.get("host") != {
        "target": "x86_64-pc-windows-msvc",
        "os": "Windows",
        "os_release": "11",
        "os_version": "10.0.26200",
        "architecture": "AMD64",
        "logical_cpu_count": 20,
        "physical_memory_bytes": 34197635072,
    }:
        fail("Windows v5 host identity drifted")

    apple_path = EVIDENCE / "report-aarch64-apple-darwin-v5.json"
    apple_report = load_json(apple_path)
    expected_tools = dict(apple_report["tools"])
    expected_tools["onnxruntime_available_providers"] = [
        "AzureExecutionProvider",
        "CPUExecutionProvider",
    ]
    if report.get("tools") != expected_tools:
        fail("Windows v5 tool or RapidOCR code identity drifted")
    if report.get("profile") != apple_report.get("profile"):
        fail("Windows v5 execution profile diverged from Apple v5")
    if report.get("patch_review") != apple_report.get("patch_review"):
        fail("Windows v5 patch-review record drifted")
    if report.get("final_evidence_review") != {
        "status": "passed",
        "reviewed_at": "2026-08-22T23:56:35Z",
        "scope": (
            "cross-target v5 identity, quality, provenance, privacy, validator "
            "drift rejection, and absence of implementation scope creep"
        ),
        "findings_closed": [
            "tracked_v5_source_hashes_not_recomputed",
            "README_v5_status_contradiction",
        ],
    }:
        fail("Windows v5 final evidence-review record drifted")

    v4_path = EVIDENCE / "report-x86_64-pc-windows-msvc.json"
    if report.get("historical_v4_report") != {
        "path": "docs/evidence/g-004/report-x86_64-pc-windows-msvc.json",
        "sha256": sha256_repository_text(v4_path),
        "status": "immutable-audit-record-not-qualification",
    }:
        fail("Windows v5 historical v4 binding drifted")
    if report.get("apple_v5_report") != {
        "path": "docs/evidence/g-004/report-aarch64-apple-darwin-v5.json",
        "sha256": sha256_repository_text(apple_path),
        "candidate_outcomes_sha256": apple_report.get("candidate_outcomes_sha256"),
    }:
        fail("Windows v5 Apple report binding drifted")

    candidates = report.get("candidates")
    expected_candidate_ids = [
        "ppocrv4-det-v6-rec-small",
        "ppocrv5-det-v6-rec-small",
        "ppocrv6-det-tiny-rec-small",
        "ppocrv6-multilingual-small",
    ]
    if (
        not isinstance(candidates, list)
        or [candidate.get("candidate_id") for candidate in candidates]
        != expected_candidate_ids
    ):
        fail("Windows v5 candidate matrix drifted")
    candidates_payload = json.dumps(
        candidates,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    observed_outcomes_sha256 = hashlib.sha256(candidates_payload).hexdigest()
    if (
        report.get("candidate_outcomes_sha256") != observed_outcomes_sha256
        or observed_outcomes_sha256
        != "f68e7da7b2dc784c72567d048fba22b0f09a4a7697cebff9270b209174dca8d9"
    ):
        fail("Windows v5 frozen candidate outcomes changed")

    fixture = load_json(FIXTURES / "fixture-manifest.json")
    fixture_lookup = {image["file"]: image for image in fixture["images"]}
    apple_lookup = {
        candidate["candidate_id"]: candidate for candidate in apple_report["candidates"]
    }
    candidate_gate_fields = (
        "stable_gate_outcomes",
        "failure_categories",
        "pass",
    )
    image_gate_fields = (
        "file",
        "fixture_sha256",
        "consumed_fixture_sha256",
        "consumed_fixture_bytes",
        "expected_region_count",
        "detected_region_count",
        "admitted_region_count",
        "exact_text_count",
        "geometry_pass_count",
        "confidence_pass_count",
        "unmatched_region_count",
        "unexpected_region_count",
        "below_unexpected_threshold_count",
        "order_pass",
        "pass",
    )
    selected = expected_candidate_ids[0]
    for candidate in candidates:
        candidate_id = candidate["candidate_id"]
        require_exact_keys(
            candidate,
            {
                "candidate_id",
                "source_identity",
                "models",
                "aggregate",
                "images",
                "stable_gate_outcomes",
                "failure_categories",
                "pass",
            },
            f"Windows v5 candidate {candidate_id}",
        )
        apple_candidate = apple_lookup.get(candidate_id)
        if apple_candidate is None:
            fail(f"Apple v5 candidate {candidate_id} is missing")
        if candidate["source_identity"] != V5_RUN_SOURCE_IDENTITY:
            fail(f"Windows v5 candidate {candidate_id} source identity drifted")
        if candidate["models"] != apple_candidate["models"]:
            fail(f"Windows v5 candidate {candidate_id} model identity diverged")
        if candidate["stable_gate_outcomes"] is not True:
            fail(f"Windows v5 candidate {candidate_id} is unstable")
        if any(
            candidate[field] != apple_candidate[field]
            for field in candidate_gate_fields
        ):
            fail(f"Windows v5 candidate gate outcome diverged for {candidate_id}")

        aggregate = require_exact_keys(
            candidate["aggregate"],
            {
                "initialization_ms",
                "suite_median_ms",
                "suite_p95_ms",
                "suite_maximum_ms",
                "process_peak_resident",
            },
            f"Windows v5 candidate {candidate_id} aggregate",
        )
        initialization = aggregate["initialization_ms"]
        median = aggregate["suite_median_ms"]
        p95 = aggregate["suite_p95_ms"]
        maximum = aggregate["suite_maximum_ms"]
        if (
            not all(
                isinstance(value, (int, float)) and value > 0
                for value in (initialization, median, p95, maximum)
            )
            or not median <= p95 <= maximum
        ):
            fail(f"Windows v5 candidate {candidate_id} timing is invalid")
        resident = require_exact_keys(
            aggregate["process_peak_resident"],
            {"status", "bytes", "source"},
            f"Windows v5 candidate {candidate_id} resident outcome",
        )
        if (
            resident["status"] != "measured"
            or not isinstance(resident["bytes"], int)
            or resident["bytes"] <= 0
            or resident["source"] != "GetProcessMemoryInfo.PeakWorkingSetSize"
        ):
            fail(f"Windows v5 candidate {candidate_id} resident outcome drifted")

        images = candidate["images"]
        apple_images = apple_candidate["images"]
        if (
            not isinstance(images, list)
            or [image.get("file") for image in images] != list(fixture_lookup)
            or len(images) != len(apple_images)
        ):
            fail(f"Windows v5 candidate {candidate_id} image matrix drifted")
        for windows_image, apple_image in zip(images, apple_images, strict=True):
            require_exact_keys(
                windows_image,
                {
                    "file",
                    "fixture_sha256",
                    "consumed_fixture_sha256",
                    "consumed_fixture_bytes",
                    "expected_region_count",
                    "detected_region_count",
                    "admitted_region_count",
                    "exact_text_count",
                    "geometry_pass_count",
                    "confidence_pass_count",
                    "unmatched_region_count",
                    "unexpected_region_count",
                    "below_unexpected_threshold_count",
                    "order_pass",
                    "minimum_matched_iou",
                    "minimum_matched_confidence",
                    "pass",
                },
                f"Windows v5 candidate {candidate_id} image",
            )
            expected_image = fixture_lookup[windows_image["file"]]
            fixture_path = FIXTURES / windows_image["file"]
            if (
                windows_image["fixture_sha256"] != expected_image["sha256"]
                or windows_image["consumed_fixture_sha256"] != expected_image["sha256"]
                or windows_image["consumed_fixture_bytes"] != fixture_path.stat().st_size
            ):
                fail(
                    f"Windows v5 candidate {candidate_id} consumed fixture drifted"
                )
            if any(
                windows_image[field] != apple_image[field]
                for field in image_gate_fields
            ):
                fail(
                    f"Windows v5 image gate outcome diverged for "
                    f"{candidate_id}/{windows_image['file']}"
                )
            for field in ("minimum_matched_iou", "minimum_matched_confidence"):
                value = windows_image[field]
                if not isinstance(value, (int, float)) or not 0.0 <= value <= 1.0:
                    fail(
                        f"Windows v5 candidate {candidate_id} "
                        f"{field} is invalid for {windows_image['file']}"
                    )

        if candidate_id == selected:
            if candidate["pass"] is not True or candidate["failure_categories"] != []:
                fail("Windows v5 selected candidate no longer passes")
        elif (
            candidate["pass"] is not False
            or candidate["failure_categories"]
            != ["ordering_mismatch", "text_mismatch", "unexpected_region"]
        ):
            fail(f"Windows v5 rejected candidate {candidate_id} outcome drifted")

    expected_decision = {
        "profile_id": "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1",
        "selected_candidate": selected,
        "source_identity_matches_apple": True,
        "model_session_vocabulary_identities_match_apple": True,
        "candidate_gate_outcomes_match_apple": True,
        "image_gate_outcomes_match_apple": True,
        "other_candidates_rejected": expected_candidate_ids[1:],
        "unresolved_evidence_gaps": [],
        "g004_status": "accepted",
    }
    if report.get("cross_target_decision") != expected_decision:
        fail("Windows v5 cross-target decision drifted")
    if report.get("privacy") != {
        "approved_expected_fixture_text_only": True,
        "unexpected_recognized_text_retained": False,
        "game_screenshot_pixels_or_text_retained": False,
        "host_paths_retained": False,
        "raw_report_private_root_enforced": True,
    }:
        fail("Windows v5 privacy declaration drifted")

    def reject_unapproved_payload(value: object) -> None:
        if isinstance(value, dict):
            if any(key in {"text", "observed", "raw"} for key in value):
                fail("Windows v5 report contains unapproved recognized payload fields")
            for nested in value.values():
                reject_unapproved_payload(nested)
        elif isinstance(value, list):
            for nested in value:
                reject_unapproved_payload(nested)
        elif isinstance(value, str) and ("/Users/" in value or "\\Users\\" in value):
            fail("Windows v5 report contains a host-local path")

    reject_unapproved_payload(report)


def validate_requirements() -> None:
    path = EVIDENCE / "tool-requirements.txt"
    required = {"rapidocr": "3.9.2", "onnxruntime": "1.29.0"}
    found: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        if line.count("==") != 1:
            fail(f"unlocked evaluation requirement: {line!r}")
        name, version = line.split("==")
        found[name.lower()] = version
    for name, version in required.items():
        if found.get(name) != version:
            fail(f"required evaluation tool {name}=={version} is not pinned")


def main() -> None:
    validate_fixture()
    validate_candidates()
    validate_v5_tracked_source_identity()
    validate_apple_report()
    validate_apple_v5_report()
    validate_requirements()
    validate_windows_report()
    validate_windows_v5_report()
    print("G-004 fixture and candidate evidence: OK")


if __name__ == "__main__":
    main()
