//! Fixed public Session inputs, row boundaries and common semantic identities.

use mado_pilot::{
    ContentDigest, MatchDefaults, MatchOptions, PixelExtent, PixelFormat, PixelRect, Suppression,
    TemplateSource,
};
use serde_json::{Value, json};

use super::fixtures::{self, Kind, PATCH, ROI, SCORE_TOLERANCE, Workload};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Case {
    RetainedHit,
    RetainedMiss,
    Group,
    DistinctGroup,
    OwnerGroup,
    Patch64,
    Turnover,
    QueryMatch,
    QueryDeadline,
    LifecycleSession,
    LifecycleIndependent,
}

impl Case {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::RetainedHit => "retained-hit",
            Self::RetainedMiss => "retained-miss",
            Self::Group => "group",
            Self::DistinctGroup => "distinct-group",
            Self::OwnerGroup => "owner-group",
            Self::Patch64 => "patch64",
            Self::Turnover => "turnover",
            Self::QueryMatch => "query-match",
            Self::QueryDeadline => "query-deadline",
            Self::LifecycleSession => "lifecycle-session",
            Self::LifecycleIndependent => "lifecycle-independent",
        }
    }
}

pub(super) const fn case(row: Workload) -> Case {
    match row.kind {
        Kind::Session(case) => case,
        _ => panic!("Session fixture requires a Session row"),
    }
}

const fn row(id: &'static str, case: Case, format: PixelFormat, roi: bool) -> Workload {
    let statistical = !matches!(
        case,
        Case::QueryDeadline | Case::LifecycleSession | Case::LifecycleIndependent
    );
    let independent = matches!(case, Case::LifecycleIndependent);
    Workload {
        id,
        category: if statistical { "session" } else { "contract" },
        improvement: false,
        statistical,
        cache_state: match case {
            Case::RetainedHit | Case::RetainedMiss | Case::Patch64 => {
                "persistent-frame-and-backend"
            }
            Case::Group | Case::DistinctGroup => "cold-engine-backend-and-frame",
            Case::OwnerGroup => "cold-with-live-compatible-roi-owner",
            Case::Turnover => "persistent-backend-new-frames",
            Case::QueryMatch | Case::QueryDeadline => "persistent-backend-fresh-query-session",
            Case::LifecycleSession => "fresh-owner-lifecycle",
            Case::LifecycleIndependent => "two-independent-backends-one-frame",
        },
        kind: Kind::Session(case),
        width: if independent { 3840 } else { 1920 },
        height: if independent { 2160 } else { 1080 },
        roi,
        k: 8,
        suppression: Suppression::DropOverlapping,
        format,
    }
}

pub(super) const WORKLOADS: [Workload; 36] = [
    row(
        "session-retained-rgba-full-hit",
        Case::RetainedHit,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "session-retained-rgba-full-miss",
        Case::RetainedMiss,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "session-retained-rgba-roi-hit",
        Case::RetainedHit,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "session-retained-rgba-roi-miss",
        Case::RetainedMiss,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "session-retained-bgra-full-hit",
        Case::RetainedHit,
        PixelFormat::Bgra8,
        false,
    ),
    row(
        "session-retained-bgra-full-miss",
        Case::RetainedMiss,
        PixelFormat::Bgra8,
        false,
    ),
    row(
        "session-retained-bgra-roi-hit",
        Case::RetainedHit,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "session-retained-bgra-roi-miss",
        Case::RetainedMiss,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "session-group8-rgba-full",
        Case::Group,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "session-group8-rgba-same-roi",
        Case::Group,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "session-group8-rgba-distinct-roi",
        Case::DistinctGroup,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "session-group8-bgra-full",
        Case::Group,
        PixelFormat::Bgra8,
        false,
    ),
    row(
        "session-group8-bgra-same-roi",
        Case::Group,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "session-group8-bgra-distinct-roi",
        Case::DistinctGroup,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "session-owner8-rgba-roi",
        Case::OwnerGroup,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "session-owner8-bgra-roi",
        Case::OwnerGroup,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "session-patch64-rgba-full",
        Case::Patch64,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "session-patch64-rgba-roi",
        Case::Patch64,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "session-patch64-bgra-full",
        Case::Patch64,
        PixelFormat::Bgra8,
        false,
    ),
    row(
        "session-patch64-bgra-roi",
        Case::Patch64,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "session-turnover-rgba-full",
        Case::Turnover,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "session-turnover-rgba-roi",
        Case::Turnover,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "session-turnover-bgra-full",
        Case::Turnover,
        PixelFormat::Bgra8,
        false,
    ),
    row(
        "session-turnover-bgra-roi",
        Case::Turnover,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "query-match-rgba-full",
        Case::QueryMatch,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "query-match-rgba-roi",
        Case::QueryMatch,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "query-match-bgra-full",
        Case::QueryMatch,
        PixelFormat::Bgra8,
        false,
    ),
    row(
        "query-match-bgra-roi",
        Case::QueryMatch,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "query-deadline-rgba-full",
        Case::QueryDeadline,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "query-deadline-rgba-roi",
        Case::QueryDeadline,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "query-deadline-bgra-full",
        Case::QueryDeadline,
        PixelFormat::Bgra8,
        false,
    ),
    row(
        "query-deadline-bgra-roi",
        Case::QueryDeadline,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "lifecycle-session-rgba-roi",
        Case::LifecycleSession,
        PixelFormat::Rgba8,
        true,
    ),
    row(
        "lifecycle-session-bgra-roi",
        Case::LifecycleSession,
        PixelFormat::Bgra8,
        true,
    ),
    row(
        "lifecycle-independent-rgba-4k",
        Case::LifecycleIndependent,
        PixelFormat::Rgba8,
        false,
    ),
    row(
        "lifecycle-independent-bgra-4k",
        Case::LifecycleIndependent,
        PixelFormat::Bgra8,
        false,
    ),
];

pub(super) fn options() -> MatchOptions {
    MatchOptions::from_defaults(defaults()).with_suppression(Suppression::DropOverlapping)
}

fn defaults() -> MatchDefaults {
    MatchDefaults::new(0.9, 8).expect("fixed authored options")
}

pub(super) const fn patch(row: Workload) -> PixelExtent {
    if matches!(case(row), Case::Patch64) {
        PixelExtent::new(64, 48)
    } else {
        PATCH
    }
}

pub(super) fn regions(row: Workload) -> Vec<PixelRect> {
    if matches!(case(row), Case::DistinctGroup) {
        (0..8)
            .map(|index| {
                let (x, y) = fixtures::origin(row.extent(), index, 0);
                PixelRect::new(x - 16, y - 16, x + 304, y + 164).expect("fixed distinct ROI")
            })
            .collect()
    } else {
        let region = if row.roi {
            PixelRect::new(ROI.0, ROI.1, ROI.2, ROI.3).expect("fixed ROI")
        } else {
            PixelRect::new(
                0,
                0,
                i32::try_from(row.width).expect("bounded width"),
                i32::try_from(row.height).expect("bounded height"),
            )
            .expect("full extent")
        };
        vec![
            region;
            if matches!(case(row), Case::Group | Case::OwnerGroup) {
                8
            } else {
                1
            }
        ]
    }
}

pub(super) fn scene(row: Workload, variant: u32, planted: bool) -> Vec<u8> {
    fixtures::scene_with_patch(row.extent(), variant, row.format, patch(row), planted)
}

pub(super) fn templates(row: Workload) -> Vec<TemplateSource> {
    let count = if matches!(
        case(row),
        Case::Group | Case::DistinctGroup | Case::OwnerGroup
    ) {
        8
    } else {
        1
    };
    fixtures::templates_with_options(patch(row), defaults(), count)
}

pub(super) fn manifest(row: Workload, smoke: bool) -> Value {
    let case = case(row);
    let (warmups, samples) = row.plan(smoke);
    let (unit, sequence, searches): (&str, &[&str], usize) = match case {
        Case::RetainedHit => ("single-search", &["H0"], 1),
        Case::RetainedMiss => ("single-search", &["M"], 1),
        Case::Group | Case::DistinctGroup | Case::OwnerGroup => ("group8", &["H0"], 8),
        Case::Patch64 => ("single-search", &["H64"], 1),
        Case::Turnover => ("turnover-cycle", &["M", "H0", "M", "H1"], 4),
        Case::QueryMatch => ("query-match", &["M", "H0", "H1"], 3),
        Case::QueryDeadline => ("query-deadline", &["M"], 1),
        Case::LifecycleSession => ("lifecycle", &["H0"], 1),
        Case::LifecycleIndependent => ("lifecycle", &["H0"], 4),
    };
    let effective_regions: Vec<_> = regions(row)
        .iter()
        .map(|region| [region.left(), region.top(), region.right(), region.bottom()])
        .collect();
    let template_order: Vec<_> =
        if matches!(case, Case::Group | Case::DistinctGroup | Case::OwnerGroup) {
            (0..8).collect()
        } else {
            vec![0]
        };
    let query = matches!(case, Case::QueryMatch | Case::QueryDeadline);
    json!({
        "id": row.id, "category": row.category, "improvement": row.improvement,
        "statistical": row.statistical, "cache_state": row.cache_state,
        "width": row.width, "height": row.height, "result_limit": row.k,
        "suppression": "DropOverlapping", "warmups": warmups, "sample_count": samples,
        "min_score": 0.9, "effective_regions": effective_regions,
        "change_policy": query.then_some("AnalysisAlways"), "format_contract": null,
        "timing_boundary": match case {
            Case::RetainedHit | Case::RetainedMiss | Case::Patch64 =>
                "public Session search through bounded scalar projection and outcome release; setup and teardown separate",
            Case::Group | Case::DistinctGroup | Case::OwnerGroup =>
                "eight public Session searches, bounded projections and outcome releases; setup, compatible mapping owner and teardown separate",
            Case::Turnover =>
                "publication/acquisition through four searches and final release per cycle; overlapping hit/miss search intervals retained separately",
            Case::QueryMatch | Case::QueryDeadline =>
                "producer release through actual query wait, terminal projection and producer join; session close, retained-terminal checks and query/outcome release separate",
            Case::LifecycleSession =>
                "complete public Session owner script including retained outcome after close and final backend release; contract only",
            Case::LifecycleIndependent =>
                "complete real Matcher owner script over one retained frame and two independent backends; contract only",
        },
        "session_contract": {
            "version": 1, "case": case.as_str(), "source_format": row.format.as_str(),
            "patch_extent": [patch(row).width(), patch(row).height()],
            "frame_sequence": sequence, "template_order": template_order,
            "unit": unit, "searches_per_unit": searches,
            "sample_strata": if matches!(case, Case::Turnover) { json!({"hit": 2, "miss": 2}) } else { json!({}) },
            "max_independent_backends": if matches!(case, Case::LifecycleIndependent) { 2 } else { 1 },
        },
    })
}

pub(super) fn corpus(legacy_templates: &[Value], legacy_scenes: &[Value]) -> Value {
    let large_templates: Vec<_> = fixtures::templates_with_options(PixelExtent::new(64, 48), defaults(), 8)
        .iter().map(|source| {
            json!({"id": source.id().as_str(), "sha256": ContentDigest::of(source.content()).to_string()})
        }).collect();
    let mut scenes = Vec::with_capacity(10);
    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        for (name, width, height, variant, planted, patch) in [
            ("H0", 1920, 1080, 0, true, PATCH),
            ("H1", 1920, 1080, 1, true, PATCH),
            ("M", 1920, 1080, 0, false, PATCH),
            ("H64", 1920, 1080, 0, true, PixelExtent::new(64, 48)),
            ("H0", 3840, 2160, 0, true, PATCH),
        ] {
            let old = legacy_scenes.iter().find(|scene| {
                planted
                    && patch == PATCH
                    && scene["width"] == width
                    && scene["height"] == height
                    && scene["variant"] == variant
                    && scene["format"]
                        == if format == PixelFormat::Rgba8 {
                            "Rgba8"
                        } else {
                            "Bgra8"
                        }
            });
            let digest = old.map_or_else(
                || {
                    ContentDigest::of(&fixtures::scene_with_patch(
                        PixelExtent::new(width, height),
                        variant,
                        format,
                        patch,
                        planted,
                    ))
                    .to_string()
                },
                |scene| {
                    scene["sha256"]
                        .as_str()
                        .expect("legacy scene digest")
                        .to_owned()
                },
            );
            scenes.push(json!({"name": name, "width": width, "height": height,
                "format": format.as_str(), "stride": u64::from(width) * 4,
                "patch_extent": [patch.width(), patch.height()], "sha256": digest}));
        }
    }
    json!({
        "generator": "template-efficiency-session-v1", "license": "repository-first-party",
        "min_score": 0.9, "result_limit": 8, "suppression": "DropOverlapping",
        "score_tolerance": SCORE_TOLERANCE,
        "small_templates": legacy_templates, "large_templates": large_templates, "scenes": scenes,
        "origins": (0..8).map(|index| {
            let (x,y) = fixtures::origin(PixelExtent::new(1920,1080), index, 0); [x,y]
        }).collect::<Vec<_>>(),
        "distinct_regions": regions(WORKLOADS[10]).iter().map(|region| {
            [region.left(), region.top(), region.right(), region.bottom()]
        }).collect::<Vec<_>>(),
        "workloads": WORKLOADS.iter().map(|&row| manifest(row, false)).collect::<Vec<_>>(),
    })
}
