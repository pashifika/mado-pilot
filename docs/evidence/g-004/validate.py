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


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
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
    if not isinstance(stages, dict):
        fail("candidate evaluation_stages are missing")
    v1 = stages.get("v1")
    v2 = stages.get("v2")
    v3 = stages.get("v3")
    v4 = stages.get("v4")
    if not isinstance(v1, dict) or set(v1.get("candidates", [])) != expected_v1:
        fail("candidate v1 screening set drifted")
    if not isinstance(v2, dict) or set(v2.get("candidates", [])) != expected_v2:
        fail("candidate v2 qualification set drifted")
    if not isinstance(v3, dict) or set(v3.get("candidates", [])) != expected_v3:
        fail("candidate v3 qualification set drifted")
    if not isinstance(v4, dict) or set(v4.get("candidates", [])) != expected_v4:
        fail("candidate v4 qualification set drifted")
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
    expected_candidates = expected_v1 | expected_v4
    if not isinstance(candidates, list) or len(candidates) != len(expected_candidates):
        fail("candidate allowlist does not match the four frozen stages")
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
    if report.get("schema_version") != 2:
        fail("Apple report schema_version must be 2")
    if report.get("qualification_stage") != "v4":
        fail("Apple report must use the hardened v4 evaluator")
    if report.get("target") != "aarch64-apple-darwin":
        fail("Apple report target must be aarch64-apple-darwin")
    if report.get("product_base_revision") != "f3608424dde88f835f35653be8113f7a2009431b":
        fail("Apple report product baseline drifted")

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
    for stage in stages:
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
        if stage.get("status") != expected_stage_statuses[stage_id]:
            fail(f"Apple stage {stage_id} status changed")

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

    current_identity = {
        "evaluator_sha256": sha256(EVIDENCE / "evaluate.py"),
        "fixture_manifest_sha256": sha256(FIXTURES / "fixture-manifest.json"),
        "candidates_sha256": sha256(EVIDENCE / "candidates.json"),
        "tool_requirements_sha256": sha256(EVIDENCE / "tool-requirements.txt"),
    }
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
    if not isinstance(decision, dict):
        fail("Apple decision record is missing")
    if decision.get("selected_candidate_for_windows") != selected:
        fail("Apple selected candidate drifted")
    if decision.get("other_v4_candidates_rejected") != [
        "ppocrv5-det-v6-rec-small",
        "ppocrv6-det-tiny-rec-small",
        "ppocrv6-multilingual-small",
    ]:
        fail("Apple rejected v4 candidate list drifted")
    if decision.get("g004_status") != "open-pending-windows":
        fail("Apple evidence must keep G-004 open pending Windows")

    privacy = report.get("privacy")
    if not isinstance(privacy, dict) or any(value is not expected for value, expected in (
        (privacy.get("approved_expected_fixture_text_only"), True),
        (privacy.get("unexpected_recognized_text_retained"), False),
        (privacy.get("game_screenshot_pixels_or_text_retained"), False),
        (privacy.get("host_paths_retained"), False),
    )):
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
    validate_apple_report()
    validate_requirements()
    print("G-004 fixture and candidate evidence: OK")


if __name__ == "__main__":
    main()
