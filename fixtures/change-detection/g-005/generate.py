# SPDX-License-Identifier: Apache-2.0
"""Generate the repository-owned G-005 RGBA8 recorded sequences."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

FIXTURE_DIR = Path(__file__).resolve().parent
REPOSITORY_ROOT = FIXTURE_DIR.parents[2]
FRAMES_DIR = FIXTURE_DIR / "frames"
WIDTH = 8
HEIGHT = 8
ROW_STRIDE = WIDTH * 4
ROI = {"x": 2, "y": 2, "width": 4, "height": 4}


def base_rgba(width: int = WIDTH, height: int = HEIGHT) -> bytes:
    pixels = bytearray()
    for y in range(height):
        for x in range(width):
            pixels.extend(
                (
                    (x * 17 + y * 3) % 256,
                    (y * 19 + x * 5) % 256,
                    ((x ^ y) * 23) % 256,
                    255,
                )
            )
    return bytes(pixels)


def with_pixels(source: bytes, width: int, changes: dict[tuple[int, int], tuple[int, int, int, int]]) -> bytes:
    pixels = bytearray(source)
    for (x, y), rgba in changes.items():
        offset = (y * width + x) * 4
        pixels[offset : offset + 4] = bytes(rgba)
    return bytes(pixels)


def frame(
    frame_id: str,
    pixels: bytes,
    *,
    stream: int,
    epoch: int,
    sequence: int,
    geometry_revision: int,
    width: int = WIDTH,
    height: int = HEIGHT,
) -> dict[str, object]:
    filename = f"{frame_id}.rgba"
    relative_path = Path("fixtures/change-detection/g-005/frames") / filename
    (REPOSITORY_ROOT / relative_path).write_bytes(pixels)
    row_stride = width * 4
    return {
        "id": frame_id,
        "path": relative_path.as_posix(),
        "width": width,
        "height": height,
        "row_stride": row_stride,
        "byte_len": len(pixels),
        "sha256": hashlib.sha256(pixels).hexdigest(),
        "stream": stream,
        "epoch": epoch,
        "sequence": sequence,
        "geometry_revision": geometry_revision,
        "pixel_width": width,
        "pixel_height": height,
    }


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def main() -> None:
    FRAMES_DIR.mkdir(parents=True, exist_ok=True)

    base = base_rgba()
    outside = with_pixels(base, WIDTH, {(0, 0): (255, 0, 255, 255)})
    one_pixel = with_pixels(base, WIDTH, {(3, 3): (255, 255, 255, 255)})
    appearance = with_pixels(
        base,
        WIDTH,
        {
            (3, 3): (255, 255, 255, 255),
            (4, 3): (255, 255, 255, 255),
            (3, 4): (255, 255, 255, 255),
            (4, 4): (255, 255, 255, 255),
        },
    )
    wider = base_rgba(10, HEIGHT)

    frames = [
        frame("no-change-0", base, stream=1, epoch=1, sequence=0, geometry_revision=1),
        frame("no-change-1", base, stream=1, epoch=1, sequence=1, geometry_revision=1),
        frame("outside-roi-0", base, stream=2, epoch=1, sequence=0, geometry_revision=1),
        frame("outside-roi-1", outside, stream=2, epoch=1, sequence=1, geometry_revision=1),
        frame("low-area-0", base, stream=3, epoch=1, sequence=0, geometry_revision=1),
        frame("low-area-1", one_pixel, stream=3, epoch=1, sequence=1, geometry_revision=1),
        frame("transient-0", base, stream=4, epoch=1, sequence=0, geometry_revision=1),
        frame("transient-1", appearance, stream=4, epoch=1, sequence=1, geometry_revision=1),
        frame("transient-2", base, stream=4, epoch=1, sequence=2, geometry_revision=1),
        frame("persistent-0", base, stream=5, epoch=1, sequence=0, geometry_revision=1),
        frame("persistent-1", appearance, stream=5, epoch=1, sequence=1, geometry_revision=1),
        frame("persistent-2", appearance, stream=5, epoch=1, sequence=3, geometry_revision=1),
        frame("geometry-0", base, stream=6, epoch=1, sequence=0, geometry_revision=1),
        frame(
            "geometry-1",
            wider,
            stream=6,
            epoch=1,
            sequence=1,
            geometry_revision=2,
            width=10,
            height=HEIGHT,
        ),
        frame("discontinuity-0", base, stream=7, epoch=1, sequence=0, geometry_revision=1),
        frame("discontinuity-1", base, stream=7, epoch=2, sequence=0, geometry_revision=1),
    ]

    sequences = [
        {"id": "no-change", "frame_ids": ["no-change-0", "no-change-1"], "roi": ROI},
        {"id": "outside-roi", "frame_ids": ["outside-roi-0", "outside-roi-1"], "roi": ROI},
        {"id": "low-area", "frame_ids": ["low-area-0", "low-area-1"], "roi": ROI},
        {
            "id": "transient-appearance",
            "frame_ids": ["transient-0", "transient-1", "transient-2"],
            "roi": ROI,
        },
        {
            "id": "persistent-appearance",
            "frame_ids": ["persistent-0", "persistent-1", "persistent-2"],
            "roi": ROI,
        },
        {"id": "geometry-change", "frame_ids": ["geometry-0", "geometry-1"], "roi": ROI},
        {
            "id": "stream-discontinuity",
            "frame_ids": ["discontinuity-0", "discontinuity-1"],
            "roi": ROI,
        },
    ]

    manifest = {
        "schema": "mado-pilot-change-sequence-manifest-v1",
        "fixture_set": "g-005-v1",
        "license": "Apache-2.0",
        "component_lengths": {"frames": 16, "sequences": 7, "transitions": 9},
        "frames": frames,
        "sequences": sequences,
    }

    rows = [
        {
            "ordinal": 0,
            "transition_id": "no-change/0",
            "sequence_id": "no-change",
            "from_frame": "no-change-0",
            "to_frame": "no-change-1",
            "compatibility": "compatible",
            "expected": "unchanged_allowed",
            "reason": "no_change",
            "must_detect": False,
        },
        {
            "ordinal": 1,
            "transition_id": "outside-roi/0",
            "sequence_id": "outside-roi",
            "from_frame": "outside-roi-0",
            "to_frame": "outside-roi-1",
            "compatibility": "compatible",
            "expected": "unchanged_allowed",
            "reason": "outside_roi_change",
            "must_detect": False,
        },
        {
            "ordinal": 2,
            "transition_id": "low-area/0",
            "sequence_id": "low-area",
            "from_frame": "low-area-0",
            "to_frame": "low-area-1",
            "compatibility": "compatible",
            "expected": "analysis_required",
            "reason": "low_area_change",
            "must_detect": True,
        },
        {
            "ordinal": 3,
            "transition_id": "transient-appearance/0",
            "sequence_id": "transient-appearance",
            "from_frame": "transient-0",
            "to_frame": "transient-1",
            "compatibility": "compatible",
            "expected": "analysis_required",
            "reason": "transient_appearance",
            "must_detect": True,
        },
        {
            "ordinal": 4,
            "transition_id": "transient-appearance/1",
            "sequence_id": "transient-appearance",
            "from_frame": "transient-1",
            "to_frame": "transient-2",
            "compatibility": "compatible",
            "expected": "analysis_required",
            "reason": "disappearance",
            "must_detect": True,
        },
        {
            "ordinal": 5,
            "transition_id": "persistent-appearance/0",
            "sequence_id": "persistent-appearance",
            "from_frame": "persistent-0",
            "to_frame": "persistent-1",
            "compatibility": "compatible",
            "expected": "analysis_required",
            "reason": "persistent_appearance",
            "must_detect": True,
        },
        {
            "ordinal": 6,
            "transition_id": "persistent-appearance/1",
            "sequence_id": "persistent-appearance",
            "from_frame": "persistent-1",
            "to_frame": "persistent-2",
            "compatibility": "compatible",
            "expected": "unchanged_allowed",
            "reason": "repeated_pixels",
            "must_detect": False,
        },
        {
            "ordinal": 7,
            "transition_id": "geometry-change/0",
            "sequence_id": "geometry-change",
            "from_frame": "geometry-0",
            "to_frame": "geometry-1",
            "compatibility": "geometry_changed",
            "expected": "analysis_required",
            "reason": "geometry_change",
            "must_detect": True,
        },
        {
            "ordinal": 8,
            "transition_id": "stream-discontinuity/0",
            "sequence_id": "stream-discontinuity",
            "from_frame": "discontinuity-0",
            "to_frame": "discontinuity-1",
            "compatibility": "stream_discontinuous",
            "expected": "analysis_required",
            "reason": "stream_discontinuity",
            "must_detect": True,
        },
    ]
    expected = {
        "schema": "mado-pilot-change-expected-v1",
        "fixture_set": "g-005-v1",
        "rows": rows,
    }

    manifest_path = FIXTURE_DIR / "fixture-manifest.json"
    expected_path = FIXTURE_DIR / "expected-rows.json"
    write_json(manifest_path, manifest)
    write_json(expected_path, expected)

    checksum_paths = [
        FIXTURE_DIR / "generate.py",
        manifest_path,
        expected_path,
        *sorted(FRAMES_DIR.glob("*.rgba")),
    ]
    checksum_lines = []
    for path in checksum_paths:
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        checksum_lines.append(f"{digest}  {path.relative_to(FIXTURE_DIR).as_posix()}")
    (FIXTURE_DIR / "SHA256SUMS").write_text("\n".join(checksum_lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
