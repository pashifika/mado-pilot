#!/usr/bin/env python3
"""Render repository-authored compact UI pixels; never consumes screenshots or OCR output."""
from __future__ import annotations

import argparse
import hashlib
import json
import unicodedata
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

FONT_SHA256 = "c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f"
FONT_REVISION = "ec626514f79f831f1ab848a82114a0ce7e2d6372"
# Order is authored top-to-bottom, then left-to-right, never inferred from a model.
CASES = (
    ("dense-tooltip.png", (640, 360), "#101923", (
        ("tooltip-title", "装備の詳細", 24, 12, 26),
        ("tooltip-level", "強化段階:7", 24, 60, 20),
        ("tooltip-attack", "攻撃力+120", 24, 100, 18),
        ("tooltip-rule-1", "行動終了時にHPを回復", 24, 140, 18),
        ("tooltip-rule-2", "次の攻撃が25%上昇", 24, 180, 18),
        ("tooltip-rule-3", "効果は3ターン継続", 24, 220, 18),
        ("tooltip-note", "同じ効果は重複しない", 24, 260, 16),
        ("tooltip-key", "[ENTER]確認", 24, 304, 18),
    )),
    ("boundary.png", (384, 192), "#1b2638", (
        ("edge-top", "境界確認", 3, 0, 24),
        ("edge-middle", "残り12秒", 230, 68, 22),
        ("edge-bottom", "戻る", 3, 146, 24),
    )),
    ("decoration.png", (512, 256), "#15202c", (
        ("decor-title", "素材一覧", 52, 18, 28),
        ("decor-count", "所持数:45", 52, 86, 22),
        ("decor-action", "選択して確認", 52, 158, 24),
    )),
    ("varied-sizes.png", (736, 360), "#172133", (
        ("large", "新しい任務", 28, 10, 40),
        ("medium", "推奨レベル20", 28, 100, 28),
        ("small", "報酬:300G", 28, 170, 20),
        ("compact", "進行状況2/5", 28, 230, 16),
        ("latin", "READY", 28, 290, 24),
    )),
    ("blank.png", (320, 160), "#172133", ()),
    ("nonuniform-negative.png", (448, 224), "#102030", ()),
)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render(font_path: Path, output: Path) -> dict:
    if digest(font_path) != FONT_SHA256:
        raise ValueError("font identity differs from the pinned G-004 Noto Sans JP source")
    images = []
    for filename, (width, height), background, rows in CASES:
        image = Image.new("RGB", (width, height), background)
        draw = ImageDraw.Draw(image)
        if filename == "nonuniform-negative.png":
            # Authored broad gradients, circles and panels; no textual glyphs.
            for x in range(width):
                draw.line((x, 0, x, height), fill=(16 + x * 36 // width, 32, 48 + x * 40 // width))
            draw.ellipse((30, 30, 146, 146), fill="#38556a")
            draw.rounded_rectangle((205, 48, 410, 180), radius=24, fill="#263951")
        elif filename == "decoration.png":
            for y in (60, 128, 200):
                draw.ellipse((12, y - 8, 28, y + 8), fill="#d0a850")
            draw.line((40, 10, 40, 242), fill="#407080", width=2)
            draw.rounded_rectangle((6, 6, 504, 248), radius=10, outline="#668899", width=2)
        regions = []
        for order, (region_id, text, x, y, size) in enumerate(rows):
            font = ImageFont.truetype(str(font_path), size=size)
            font.set_variation_by_axes([400])
            draw.text((x, y), text, font=font, fill="#f4f1db", stroke_width=0)
            left, top, right, bottom = draw.textbbox((x, y), text, font=font, stroke_width=0)
            left, top = max(0, left - 3), max(0, top - 3)
            right, bottom = min(width, right + 3), min(height, bottom + 3)
            regions.append({"id": region_id, "order": order,
                            "text_nfc": unicodedata.normalize("NFC", text),
                            "source_relative_quad": [[left / width, top / height],
                                                     [right / width, top / height],
                                                     [right / width, bottom / height],
                                                     [left / width, bottom / height]]})
        path = output / filename
        image.save(path, format="PNG", optimize=False, compress_level=9)
        images.append({"file": filename, "sha256": digest(path), "width": width,
                       "height": height, "regions": regions})
    return {"schema_version": 1, "fixture_profile_id": "compact-ui-v1", "license": "Apache-2.0",
            "authorship": "Independently authored for MadoPilot; no private pixels, labels, or digests",
            "font": {"name": "Noto Sans JP", "source_revision": FONT_REVISION,
                     "source_url": f"https://github.com/google/fonts/blob/{FONT_REVISION}/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf",
                     "sha256": FONT_SHA256, "license": "OFL-1.1", "bytes_bundled": False},
            "oracle": {"text": "exact NFC with outer whitespace trimmed; no case/width/fuzzy folding",
                       "region_count": "exact", "ordering": "authored manifest order",
                       "geometry_space": "source-relative quadrilateral", "minimum_iou": 0.5,
                       "maximum_center_delta_x": 0.025, "maximum_center_delta_y": 0.025,
                       "unexpected_region_threshold": 0.5,
                       "confidence": "finite model-local [0,1]; no universal matching floor",
                       "stability": "identical text, order, geometry and confidence within an implementation"},
            "images": images}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--font", type=Path, required=True)
    args = parser.parse_args()
    output = Path(__file__).resolve().parent
    manifest = render(args.font, output)
    (output / "fixture-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (output / "SHA256SUMS").write_text("".join(f"{row['sha256']}  {row['file']}\n" for row in manifest["images"]), encoding="utf-8")


if __name__ == "__main__":
    main()
