#!/usr/bin/env python3
"""Materialize fixed OCR watcher replay pixels; never loads a model or captures a screen."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

from PIL import Image, __version__ as pillow_version

ROOT = Path(__file__).resolve().parents[2]
IMAGE = ROOT / "fixtures/ocr/g-004/hud.png"
IMAGE_SHA256 = "10f0163cf298453e55922cc6104ee3066f59a328a55e9bac6c04f24d36c9e288"
WIDTH, HEIGHT = 960, 540
TEXT = ("魔導士", "Lv.42", "HP1234/5678", "MP98%", "クエスト", "[A-7]", "次へ>", "READY!")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def frame(file: str, instant: int, *, retina: bool = False) -> dict:
    value = {
        "pixels": file,
        "width": WIDTH,
        "height": HEIGHT,
        "format": "bgra8",
        "captured_at_nanos": instant,
        "continuity": "continuous",
    }
    if retina:
        value["placement"] = {
            "desktop_origin": [100.0, 50.0],
            "logical_size": [480.0, 270.0],
            "scale": [2.0, 2.0],
        }
    return value


def materialize(output: Path) -> dict:
    if pillow_version != "12.3.0":
        raise ValueError("Pillow 12.3.0 is required for this fixed fixture conversion")
    if digest(IMAGE.read_bytes()) != IMAGE_SHA256:
        raise ValueError("the repository-owned source image digest differs")
    with Image.open(IMAGE) as image:
        if image.size != (WIDTH, HEIGHT):
            raise ValueError("the source image extent differs")
        pixels = image.convert("RGBA").tobytes("raw", "BGRA")
    if len(pixels) != WIDTH * HEIGHT * 4:
        raise ValueError("the converted pixel extent differs")
    blank = bytes((255, 255, 255, 255)) * (WIDTH * HEIGHT)
    manifest = {
        "schema_version": 1,
        "targets": [
            {"name": "ocr-transition", "frames": [frame("blank.bgra", 0), frame("hud.bgra", 16_000_000)]},
            {"name": "ocr-negative", "frames": [frame("blank.bgra", 0)]},
            {"name": "ocr-retina", "frames": [frame("hud.bgra", 0, retina=True)]},
        ],
    }
    oracle = {
        "schema_version": 1,
        "profile": "ocr-text-watch-replay-v2",
        "license": "Apache-2.0",
        "rendered_font_license": "OFL-1.1; Noto Sans JP; no font bytes included",
        "source": "fixtures/ocr/g-004/hud.png",
        "source_sha256": IMAGE_SHA256,
        "pillow_version": pillow_version,
        "format": "bgra8",
        "width": WIDTH,
        "height": HEIGHT,
        "literal_nfc": TEXT[0],
        "minimum_confidence": 0.0,
        "stability": "Immediate",
        "expected_regions_nfc": TEXT,
        "satisfying_indexes": [0],
        "satisfying_source_box": [53.0, 60.0, 203.0, 112.0],
        "minimum_box_iou": 0.5,
        "maximum_center_delta_fraction": [0.025, 0.025],
        "targets": {
            "ocr-transition": {"terminal": "Matched", "sequence": 1, "epoch": 0, "geometry": 0, "confirmations": 1},
            "ocr-negative": {"terminal": "SessionClosed", "matched": False},
            "ocr-retina": {"terminal": "Matched", "sequence": 0, "epoch": 0, "geometry": 0, "confirmations": 1, "output_space": "TargetLogical"},
        },
        "latest_wins": "A replaced negative may remain unanalysed; the sole positive transition frame is exact sequence 1. No intermediate analysis count is assumed.",
        "files": {"hud.bgra": {"bytes": len(pixels), "sha256": digest(pixels)}, "blank.bgra": {"bytes": len(blank), "sha256": digest(blank)}},
    }
    output.mkdir(parents=True, exist_ok=False)
    for name, value in (("hud.bgra", pixels), ("blank.bgra", blank)):
        with (output / name).open("xb") as stream:
            stream.write(value)
    for name, value in (("madopilot-replay.json", manifest), ("oracle.json", oracle)):
        with (output / name).open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
    return {"profile": oracle["profile"], "files": oracle["files"], "targets": 3}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, help="new, nonexisting corpus directory")
    args = parser.parse_args()
    try:
        receipt = materialize(args.output)
    except (OSError, ValueError):
        print("OCR watcher corpus generation failed; output is not qualified", file=sys.stderr)
        return 1
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
