//! Repository-owned integer fixtures and an independent score-prefix oracle.

use std::sync::Arc;

use mado_pilot::{
    ContentDigest, CoordinateSpace, MatchDefaults, PixelExtent, PixelFormat, Suppression,
    TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
};
use mado_pilot_backend_opencv::benchmark_instrumentation::SelectionRequest;
use mado_pilot_testkit::{match_fixtures, png};
use serde_json::{Value, json};

pub(super) const PATCH: PixelExtent = PixelExtent::new(12, 10);
pub(super) const ROI: (i32, i32, i32, i32) = (64, 48, 384, 228);
pub(super) const SCORE_TOLERANCE: f64 = 1e-5;

#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Single,
    Selection,
    ColdBatch,
    LiveOwnerBatch,
    WarmBatch,
    Concurrent,
    DifferentFrame,
    EqualStamp,
    ExpiredMapping,
    Eviction,
    Oversized,
    RoiFormatTransition,
    WatchAnalysisAlways,
    WatchExactRgba,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Workload {
    pub id: &'static str,
    pub category: &'static str,
    pub improvement: bool,
    pub statistical: bool,
    pub cache_state: &'static str,
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    pub roi: bool,
    pub k: usize,
    pub suppression: Suppression,
}

impl Workload {
    pub(super) fn manifest(self, smoke: bool) -> Value {
        let (warmups, samples) = self.plan(smoke);
        let full = [
            0,
            0,
            i32::try_from(self.width).expect("bounded width"),
            i32::try_from(self.height).expect("bounded height"),
        ];
        let regions = [
            match self.kind {
                Kind::WatchAnalysisAlways | Kind::WatchExactRgba => [16, 8, 48, 32],
                _ if self.roi => [ROI.0, ROI.1, ROI.2, ROI.3],
                _ => full,
            },
            [72, 56, 376, 220],
            full,
        ];
        let region_count = match self.kind {
            Kind::Selection => 0,
            Kind::RoiFormatTransition => 3,
            _ => 1,
        };
        json!({
            "id": self.id, "category": self.category, "improvement": self.improvement,
            "statistical": self.statistical, "cache_state": self.cache_state,
            "width": self.width, "height": self.height, "result_limit": self.k,
            "suppression": if self.suppression == Suppression::KeepAll { "KeepAll" } else { "DropOverlapping" },
            "warmups": warmups, "sample_count": samples,
            "min_score": match self.kind {
                Kind::Selection => 0.8,
                Kind::WatchAnalysisAlways | Kind::WatchExactRgba => 0.95,
                _ => 0.99,
            },
            "effective_regions": &regions[..region_count],
            "change_policy": match self.kind {
                Kind::WatchAnalysisAlways => Some("AnalysisAlways"),
                Kind::WatchExactRgba => Some("ExactRgba"),
                _ => None,
            },
            "format_contract": matches!(self.kind, Kind::RoiFormatTransition).then(super::formats::manifest),
            "timing_boundary": match self.kind {
                Kind::Selection => "production selection; scratch restoration and reference oracle excluded",
                Kind::Concurrent => "barrier release through two public Matcher calls and joins; worker creation excluded",
                Kind::WatchAnalysisAlways | Kind::WatchExactRgba => "public fixed-policy watcher lifecycle including final-owner teardown; contract only",
                Kind::RoiFormatTransition => "finite public mapping/matching and owner checkpoints; non-statistical",
                _ => "all public Matcher searches; fixture, key creation, retained ROI owner and teardown excluded",
            },
        })
    }

    pub(super) fn plan(self, smoke: bool) -> (usize, usize) {
        if smoke || !self.statistical {
            (1, 3)
        } else {
            (20, 200)
        }
    }

    pub(super) fn extent(self) -> PixelExtent {
        PixelExtent::new(self.width, self.height)
    }

    pub(super) fn selection(self) -> SelectionRequest {
        SelectionRequest {
            width: self.width as usize,
            height: self.height as usize,
            template_width: PATCH.width() as usize,
            template_height: PATCH.height() as usize,
            min_score: 0.8,
            candidate_budget: self.k,
            suppression: self.suppression,
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one fixed manifest table, not a configurable builder"
)]
const fn row(
    id: &'static str,
    category: &'static str,
    improvement: bool,
    statistical: bool,
    cache_state: &'static str,
    kind: Kind,
    width: u32,
    height: u32,
    roi: bool,
    k: usize,
    suppression: Suppression,
) -> Workload {
    Workload {
        id,
        category,
        improvement,
        statistical,
        cache_state,
        kind,
        width,
        height,
        roi,
        k,
        suppression,
    }
}

pub(super) const WORKLOADS: [Workload; 21] = [
    row(
        "single-small96x64",
        "guard",
        false,
        true,
        "fresh-frame-and-backend",
        Kind::Single,
        96,
        64,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "single-1080p",
        "guard",
        false,
        true,
        "fresh-frame-and-backend",
        Kind::Single,
        1920,
        1080,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "single-4k",
        "guard",
        false,
        true,
        "fresh-frame-and-backend",
        Kind::Single,
        3840,
        2160,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "selection-k1-large",
        "selection",
        false,
        true,
        "restored-score-map",
        Kind::Selection,
        3840,
        2160,
        false,
        1,
        Suppression::KeepAll,
    ),
    row(
        "selection-k16-dense1080p-keepall",
        "selection",
        true,
        true,
        "restored-score-map",
        Kind::Selection,
        1920,
        1080,
        false,
        16,
        Suppression::KeepAll,
    ),
    row(
        "selection-k16-dense1080p-dropoverlapping",
        "selection",
        true,
        true,
        "restored-score-map",
        Kind::Selection,
        1920,
        1080,
        false,
        16,
        Suppression::DropOverlapping,
    ),
    row(
        "selection-k256-small-dense-tied",
        "selection",
        true,
        true,
        "restored-score-map",
        Kind::Selection,
        96,
        64,
        false,
        256,
        Suppression::KeepAll,
    ),
    row(
        "selection-k256-large-sparse",
        "selection",
        true,
        true,
        "restored-score-map",
        Kind::Selection,
        3840,
        2160,
        false,
        256,
        Suppression::DropOverlapping,
    ),
    row(
        "batch8-cold-full1080p",
        "repeated-preparation",
        true,
        true,
        "fresh-frame-and-backend-per-batch",
        Kind::ColdBatch,
        1920,
        1080,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "batch8-cold-roi320x180",
        "repeated-preparation",
        true,
        true,
        "fresh-frame-and-backend-per-batch-released-mapping-owners",
        Kind::ColdBatch,
        1920,
        1080,
        true,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "batch8-live-owner-roi320x180",
        "repeated-preparation",
        false,
        true,
        "fresh-frame-and-backend-per-batch-one-live-roi-owner",
        Kind::LiveOwnerBatch,
        1920,
        1080,
        true,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "batch8-warm-full1080p",
        "repeated-preparation",
        false,
        true,
        "same-live-frame-and-backend-prewarmed-before-first-iteration",
        Kind::WarmBatch,
        1920,
        1080,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "concurrent2-cold-full1080p",
        "guard",
        false,
        true,
        "fresh-shared-frame-and-backend-two-barrier-started-workers",
        Kind::Concurrent,
        1920,
        1080,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "contract-different-frame",
        "contract",
        false,
        false,
        "two-live-distinct-frame-identities-one-backend",
        Kind::DifferentFrame,
        1920,
        1080,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "contract-equal-stamp",
        "contract",
        false,
        false,
        "two-live-distinct-frame-instances-equal-numeric-stamps",
        Kind::EqualStamp,
        1920,
        1080,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "contract-expired-mapping",
        "contract",
        false,
        false,
        "same-frame-roi-all-prior-mapping-owners-released",
        Kind::ExpiredMapping,
        1920,
        1080,
        true,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "contract-eviction",
        "contract",
        false,
        false,
        "three-live-frame-keys-A-B-C-A-one-backend",
        Kind::Eviction,
        1920,
        1080,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "contract-oversized-bgr",
        "contract",
        false,
        false,
        "fresh-input-BGR-payload-above-33554432-bytes",
        Kind::Oversized,
        4096,
        2732,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "contract-roi-format-transition",
        "contract",
        false,
        false,
        "ordered-direct-layouts-forced-BGRA-persistent-hit-and-active-eviction",
        Kind::RoiFormatTransition,
        1920,
        1080,
        true,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "watch-analysis-always",
        "contract",
        false,
        false,
        "fresh-public-watcher-fixed-policy-and-final-owner-release",
        Kind::WatchAnalysisAlways,
        96,
        64,
        false,
        1,
        Suppression::DropOverlapping,
    ),
    row(
        "watch-exact-rgba",
        "contract",
        false,
        false,
        "fresh-public-watcher-fixed-policy-and-final-owner-release",
        Kind::WatchExactRgba,
        96,
        64,
        false,
        1,
        Suppression::DropOverlapping,
    ),
];

fn color(seed: u32, x: u32, y: u32) -> [u8; 3] {
    let mut value = seed ^ x.wrapping_mul(0x9e37_79b9) ^ y.wrapping_mul(0x85eb_ca6b);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    let bytes = value.to_le_bytes();
    [bytes[0], bytes[1], bytes[2]]
}

pub(super) fn origin(extent: PixelExtent, template: usize, variant: u32) -> (i32, i32) {
    let template = i32::try_from(template).expect("eight fixture templates");
    let variant = i32::try_from(variant).expect("three fixture variants");
    let (x, y) = if extent.width() == 96 {
        (20, 12)
    } else {
        (80 + (template % 4) * 72, 64 + (template / 4) * 72)
    };
    (x + variant * 3, y + variant * 2)
}

pub(super) fn scene(extent: PixelExtent, variant: u32, format: PixelFormat) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 4);
    for y in 0..extent.height() {
        for x in 0..extent.width() {
            let [r, g, b] = color(0x3141_5926, x, y);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    for template in 0..if extent.width() == 96 { 1 } else { 8 } {
        let (left, top) = origin(extent, template, variant);
        let left = u32::try_from(left).expect("nonnegative planted column");
        let top = u32::try_from(top).expect("nonnegative planted row");
        let seed = 0xabc0_0000 + u32::try_from(template).expect("eight templates");
        for y in 0..PATCH.height() {
            for x in 0..PATCH.width() {
                let offset = (((top + y) * extent.width() + left + x) * 4) as usize;
                pixels[offset..offset + 3].copy_from_slice(&color(seed, x, y));
            }
        }
    }
    if format == PixelFormat::Bgra8 {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    } else {
        assert_eq!(format, PixelFormat::Rgba8);
    }
    pixels
}

pub(super) fn templates() -> Vec<TemplateSource> {
    (0..8)
        .map(|index| {
            let mut rgb = Vec::with_capacity((PATCH.width() * PATCH.height() * 3) as usize);
            for y in 0..PATCH.height() {
                for x in 0..PATCH.width() {
                    rgb.extend_from_slice(&color(0xabc0_0000 + index, x, y));
                }
            }
            TemplateSource::new(TemplateSourceRequest {
                id: TemplateId::new(format!("efficiency.patch{index}")).expect("fixed template ID"),
                encoding: TemplateEncoding::Png,
                extent: PATCH,
                space: CoordinateSpace::CapturePixels,
                defaults: MatchDefaults::new(0.99, 1).expect("fixed matching options"),
                content: Arc::from(png::encode_rgb(PATCH.width(), PATCH.height(), &rgb)),
            })
            .expect("generated PNG source")
        })
        .collect()
}

pub(super) fn score_map(row: Workload) -> Vec<f32> {
    let mut scores: Vec<f32> = (0..row.width as usize * row.height as usize)
        .map(|offset| {
            if row.id == "selection-k256-large-sparse" {
                0.125
            } else {
                match offset % 257 {
                    0 => f32::NAN,
                    1 => f32::INFINITY,
                    2 => f32::NEG_INFINITY,
                    3 => -0.0,
                    4 => -0.5,
                    5 => 1.25,
                    6 => 1.0,
                    _ => 0.8 + (offset % 17) as f32 / 100.0,
                }
            }
        })
        .collect();
    if row.id == "selection-k256-large-sparse" {
        let width = row.width as usize;
        for (x, y, value) in [
            (17, 19, 1.25),
            (18, 19, 1.0),
            (900, 700, 0.9),
            (2700, 1600, 0.8),
        ] {
            scores[y * width + x] = value;
        }
        scores[0] = f32::NAN;
        scores[1] = f32::INFINITY;
    }
    scores
}

pub(super) fn score_oracle(source: &[f32], request: SelectionRequest) -> Vec<(usize, usize, f64)> {
    let mut remaining = source.to_vec();
    let mut expected = Vec::new();
    while expected.len() < request.candidate_budget {
        let winner = remaining
            .iter()
            .enumerate()
            .filter(|&(_, &raw)| raw.is_finite())
            .map(|(index, &raw)| (index, f64::from(raw).clamp(0.0, 1.0)))
            .filter(|(_, score)| *score >= request.min_score)
            .max_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .expect("finite public scores")
                    .then_with(|| right.0.cmp(&left.0))
            });
        let Some((offset, score)) = winner else { break };
        let (x, y) = (offset % request.width, offset / request.width);
        expected.push((x, y, score));
        if request.suppression == Suppression::KeepAll {
            remaining[offset] = f32::NAN;
        } else {
            for (index, value) in remaining.iter_mut().enumerate() {
                if (index % request.width).abs_diff(x) < request.template_width
                    && (index / request.width).abs_diff(y) < request.template_height
                {
                    *value = f32::NAN;
                }
            }
        }
    }
    expected
}

pub(super) fn manifest() -> (String, Value) {
    let template_digests: Vec<_> = templates().iter().map(|source| {
        json!({"id": source.id().as_str(), "sha256": ContentDigest::of(source.content()).to_string()})
    }).collect();
    let mut scenes = Vec::new();
    for (width, height, variant, bgra) in [
        (96, 64, 0, false),
        (1920, 1080, 0, false),
        (3840, 2160, 0, false),
        (1920, 1080, 1, false),
        (1920, 1080, 2, false),
        (4096, 2732, 0, false),
        (1920, 1080, 0, true),
    ] {
        let format = if bgra {
            PixelFormat::Bgra8
        } else {
            PixelFormat::Rgba8
        };
        scenes.push(json!({"width": width, "height": height, "variant": variant,
            "format": if bgra { "Bgra8" } else { "Rgba8" },
            "sha256": ContentDigest::of(&scene(PixelExtent::new(width, height), variant, format)).to_string()}));
    }
    let scores: Vec<_> = WORKLOADS
        .iter()
        .filter(|row| matches!(row.kind, Kind::Selection))
        .map(|&row| {
            let values = score_map(row);
            let mut encoded = Vec::with_capacity(values.len() * 4);
            for value in values {
                encoded.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            json!({"id": row.id, "sha256": ContentDigest::of(&encoded).to_string()})
        })
        .collect();
    let fixture = json!({
        "generator": "template-efficiency-integer-v2-direct-formats", "license": "repository-first-party",
        "background_seed": 0x3141_5926_u32, "patch_seed_base": 0xabc0_0000_u32,
        "patch_width": PATCH.width(), "patch_height": PATCH.height(),
        "roi": [ROI.0, ROI.1, ROI.2, ROI.3], "score_tolerance": SCORE_TOLERANCE,
        "templates": template_digests, "scenes": scenes, "score_maps": scores,
        "direct_formats": {
            "helper_source_sha256": ContentDigest::of(include_bytes!("template_efficiency_formats.rs")).to_string(),
            "fixture_source_sha256": ContentDigest::of(include_bytes!("template_efficiency_fixtures.rs")).to_string(),
            "protocol": super::formats::manifest(),
            "padded_row_bytes": 16, "padding_value": 0xA5,
        },
        "watcher": {
            "helper_source_sha256": ContentDigest::of(include_bytes!("template_efficiency_watch.rs")).to_string(),
            "generator_source_sha256": ContentDigest::of(include_bytes!("../../../support/testkit/src/match_fixtures.rs")).to_string(),
            "scene_rgba_sha256": ContentDigest::of(&match_fixtures::scene_pixels(PixelFormat::Rgba8)).to_string(),
            "background_rgba_sha256": ContentDigest::of(&match_fixtures::background_pixels(PixelFormat::Rgba8)).to_string(),
            "patch_png_sha256": ContentDigest::of(match_fixtures::planted_template("watch-present").content()).to_string(),
            "absent_png_sha256": ContentDigest::of(match_fixtures::absent_template("watch-absent").content()).to_string(),
            "roi": [16, 8, 48, 32], "min_score": 0.95, "consecutive_frames": 2,
        },
    });
    let digest = ContentDigest::of(&serde_json::to_vec(&fixture).expect("finite fixture manifest"))
        .to_string();
    (digest, fixture)
}
