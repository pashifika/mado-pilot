#!/usr/bin/env python3
"""Generate the immutable G-004 synthetic OCR fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import unicodedata
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

FONT_SHA256 = "c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f"
FONT_SOURCE_REVISION = "ec626514f79f831f1ab848a82114a0ce7e2d6372"
FONT_SOURCE_URL = (
    "https://github.com/google/fonts/blob/"
    f"{FONT_SOURCE_REVISION}/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf"
)

FIXTURES = (
    {
        "file": "hud.png",
        "size": (960, 540),
        "background": "#111827",
        "bands": (
            (24, 24, 936, 126, "#1f2937"),
            (24, 142, 936, 244, "#172554"),
            (24, 260, 936, 362, "#312e81"),
            (24, 378, 936, 516, "#134e4a"),
        ),
        "regions": (
            ("hud-name", "魔導士", 56, 48, 48, "#f9fafb"),
            ("hud-level", "Lv.42", 708, 54, 42, "#fde68a"),
            ("hud-hp", "HP1234/5678", 56, 162, 42, "#fecaca"),
            ("hud-mp", "MP98%", 720, 166, 38, "#bfdbfe"),
            ("hud-quest", "クエスト", 56, 278, 44, "#e0e7ff"),
            ("hud-code", "[A-7]", 738, 286, 36, "#c4b5fd"),
            ("hud-next", "次へ>", 56, 402, 42, "#ccfbf1"),
            ("hud-ready", "READY!", 696, 410, 34, "#a7f3d0"),
        ),
    },
    {
        "file": "menu.png",
        "size": (720, 480),
        "background": "#f8fafc",
        "bands": (
            (28, 24, 692, 112, "#0f172a"),
            (28, 128, 692, 216, "#e2e8f0"),
            (28, 232, 692, 320, "#dbeafe"),
            (28, 336, 692, 448, "#ede9fe"),
        ),
        "regions": (
            ("menu-title", "設定", 56, 44, 44, "#ffffff"),
            ("menu-title-latin", "MENU", 526, 50, 34, "#93c5fd"),
            ("menu-volume", "音量", 64, 148, 38, "#0f172a"),
            ("menu-volume-value", "75%", 548, 150, 36, "#1d4ed8"),
            ("menu-subtitle", "字幕", 64, 252, 38, "#172554"),
            ("menu-subtitle-value", "[ON]", 520, 254, 34, "#1e40af"),
            ("menu-back", "戻る", 64, 358, 38, "#2e1065"),
            ("menu-back-key", "ESC", 540, 364, 32, "#6d28d9"),
        ),
    },
    {
        "file": "status.png",
        "size": (640, 360),
        "background": "#020617",
        "bands": (
            (20, 20, 620, 108, "#052e16"),
            (20, 126, 620, 214, "#422006"),
            (20, 232, 620, 340, "#3b0764"),
        ),
        "regions": (
            ("status-save", "セーブ完了", 40, 42, 32, "#dcfce7"),
            ("status-slot", "SLOT03", 478, 48, 26, "#86efac"),
            ("status-money", "所持金", 40, 148, 32, "#fef3c7"),
            ("status-money-value", "12,345G", 430, 152, 28, "#fbbf24"),
            ("status-confirm", "確認#1", 40, 258, 30, "#f3e8ff"),
            ("status-confirm-key", "[OK]", 478, 264, 26, "#d8b4fe"),
        ),
    },
    {
        "file": "tooltip-v3.png",
        "size": (1440, 720),
        "background": "#071018",
        "bands": (
            (48, 36, 1392, 684, "#0b1724"),
            (96, 72, 690, 648, "#17324a"),
            (738, 72, 1344, 304, "#202225"),
            (738, 326, 1344, 470, "#202225"),
            (738, 492, 1344, 636, "#202225"),
        ),
        "regions": (
            ("tooltip-card-title", "戦術カード", 140, 108, 32, "#fef3c7"),
            ("tooltip-rule-title", "一点照準", 774, 102, 30, "#f8fafc"),
            ("tooltip-card-type", "強化", 140, 158, 28, "#bbf7d0"),
            ("tooltip-rule-line-1", "ターン終了時、HPが回復", 774, 158, 24, "#e5e7eb"),
            ("tooltip-rule-line-2", "追加攻撃35%", 774, 204, 24, "#fde68a"),
            ("tooltip-rule-line-3", "最大60重複", 774, 250, 24, "#fde68a"),
            ("tooltip-unique-title", "唯一", 774, 350, 28, "#fbbf24"),
            ("tooltip-unique-line", "同じカードは1枚まで", 774, 404, 24, "#e5e7eb"),
            ("tooltip-opening-title", "開戦", 774, 516, 28, "#fbbf24"),
            ("tooltip-card-rule", "ドロー2", 140, 548, 30, "#f8fafc"),
            ("tooltip-opening-line", "戦闘開始時に配置", 774, 570, 24, "#e5e7eb"),
        ),
    },
    {
        "file": "mission.png",
        "size": (1440, 720),
        "background": "#e8edf3",
        "bands": (
            (32, 24, 1408, 696, "#f4f6f8"),
            (48, 32, 410, 92, "#ffffff"),
            (850, 52, 1370, 128, "#e2e8f0"),
            (850, 154, 1370, 230, "#ffffff"),
            (850, 246, 1370, 322, "#ffffff"),
            (850, 338, 1370, 414, "#ffffff"),
            (850, 430, 1370, 506, "#ffffff"),
            (850, 522, 1370, 602, "#ffffff"),
            (850, 620, 1110, 684, "#f97316"),
            (1128, 620, 1370, 684, "#9333ea"),
        ),
        "regions": (
            ("mission-team", "チーム編成", 74, 44, 32, "#111827"),
            ("mission-material", "パートナーレベルアップ素材", 880, 76, 28, "#374151"),
            ("mission-difficulty", "難易度6", 884, 176, 26, "#c2410c"),
            ("mission-recommended", "推奨Lv.50", 884, 268, 26, "#374151"),
            ("mission-monster", "登場モンスター", 884, 360, 26, "#374151"),
            ("mission-loot", "獲得できる戦利品", 884, 452, 26, "#374151"),
            ("mission-efficiency", "戦闘効率:5", 884, 548, 26, "#374151"),
            ("mission-enter", "次の任務へ", 900, 634, 26, "#ffffff"),
            ("mission-confirm", "確認", 1210, 634, 26, "#ffffff"),
        ),
    },
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_relative_quad(box: tuple[int, int, int, int], width: int, height: int) -> list[list[float]]:
    left, top, right, bottom = box
    return [
        [round(left / width, 8), round(top / height, 8)],
        [round(right / width, 8), round(top / height, 8)],
        [round(right / width, 8), round(bottom / height, 8)],
        [round(left / width, 8), round(bottom / height, 8)],
    ]


def render(font_path: Path, output_dir: Path) -> dict[str, object]:
    if sha256(font_path) != FONT_SHA256:
        raise SystemExit(f"font digest mismatch: expected {FONT_SHA256}")

    images = []
    for fixture in FIXTURES:
        width, height = fixture["size"]
        image = Image.new("RGB", (width, height), fixture["background"])
        draw = ImageDraw.Draw(image)
        for left, top, right, bottom, color in fixture["bands"]:
            draw.rounded_rectangle((left, top, right, bottom), radius=14, fill=color)

        regions = []
        for order, (region_id, text, x, y, size, color) in enumerate(fixture["regions"]):
            font = ImageFont.truetype(str(font_path), size=size)
            font.set_variation_by_axes([400])
            draw.text((x, y), text, font=font, fill=color, stroke_width=0)
            ink = draw.textbbox((x, y), text, font=font, stroke_width=0)
            padded = (
                max(0, ink[0] - 3),
                max(0, ink[1] - 3),
                min(width, ink[2] + 3),
                min(height, ink[3] + 3),
            )
            regions.append(
                {
                    "id": region_id,
                    "order": order,
                    "text_nfc": unicodedata.normalize("NFC", text),
                    "source_relative_quad": source_relative_quad(padded, width, height),
                }
            )

        output_path = output_dir / fixture["file"]
        image.save(output_path, format="PNG", optimize=False, compress_level=9)
        images.append(
            {
                "file": fixture["file"],
                "sha256": sha256(output_path),
                "width": width,
                "height": height,
                "regions": regions,
            }
        )

    return {
        "schema_version": 1,
        "fixture_profile_id": "g-004-japanese-ui-v3",
        "supersedes_fixture_profile_id": "g-004-japanese-ui-v2",
        "license": "Apache-2.0",
        "font": {
            "name": "Noto Sans JP",
            "source_revision": FONT_SOURCE_REVISION,
            "source_url": FONT_SOURCE_URL,
            "sha256": FONT_SHA256,
            "license": "OFL-1.1",
            "bytes_bundled": False,
        },
        "language_profile": {
            "scripts": ["Japanese", "basic Latin", "ASCII digits", "declared UI symbols"],
            "orientation": "horizontal-only",
            "vertical_text_supported": False,
            "normalization": {
                "unicode": "NFC",
                "trim_leading_trailing_whitespace": True,
                "internal_whitespace": "preserve",
                "case_fold": False,
                "width_fold": False,
            },
        },
        "oracle": {
            "text": "exact normalized UTF-8 per region",
            "region_count": "exact",
            "ordering": "RapidOCR v3.9.2 detector order, fixed explicitly in manifest order",
            "geometry_space": "source-relative quadrilateral",
            "minimum_iou": 0.5,
            "maximum_center_delta_x": 0.025,
            "maximum_center_delta_y": 0.025,
            "confidence": {
                "meaning": "RapidOCR CTC mean of retained non-blank token maxima after duplicate removal",
                "valid_range": [0.0, 1.0],
                "deterministic_across_measured_passes": True,
                "universal_hard_floor": None,
            },
            "unexpected_region_threshold": 0.5,
            "cross_target": "all normalized text, region count, ordering, and pass/fail outcomes identical; confidence values may differ",
        },
        "images": images,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--font", required=True, type=Path)
    args = parser.parse_args()

    output_dir = Path(__file__).resolve().parent
    manifest = render(args.font, output_dir)
    manifest_path = output_dir / "fixture-manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    pngs = [output_dir / image["file"] for image in manifest["images"]]
    sums = "".join(f"{sha256(path)}  {path.name}\n" for path in pngs)
    (output_dir / "SHA256SUMS").write_text(sums, encoding="utf-8")


if __name__ == "__main__":
    main()
