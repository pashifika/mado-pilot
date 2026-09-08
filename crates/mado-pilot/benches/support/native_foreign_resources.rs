//! Compiled, opt-in sampled OS resource envelopes for the foreign controller.
//!
//! Declarations and a campaign-record digest do not authenticate a physical host,
//! source checkout, headers, dependencies or build inventory. Those remain an
//! external frozen-campaign review, separate from exact executable admission.

use std::fmt::Write as _;

use super::{Artifact, Checked, Fact, FactKey, Failure, Ledger, Number, Outcome, Phase};

const METADATA_LIMIT: usize = 256;
const COMPARISON_COUNT: usize = 27;

pub(super) struct ResourceRequest {
    pub(super) profile_id: String,
    pub(super) hardware: String,
    pub(super) os_version: String,
    pub(super) topology: String,
    pub(super) source_commit: String,
    pub(super) source_tree: String,
    pub(super) runner_sha256: String,
    pub(super) context_sha256: String,
}

impl ResourceRequest {
    pub(super) const KEYS: [&str; 8] = [
        "--foreign-resource-profile",
        "--foreign-resource-hardware",
        "--foreign-resource-os-version",
        "--foreign-resource-topology",
        "--foreign-resource-source-commit",
        "--foreign-resource-source-tree",
        "--foreign-resource-runner-sha256",
        "--foreign-resource-context-sha256",
    ];

    pub(super) fn parse(values: [Option<&str>; 8]) -> Checked<Option<Self>> {
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [
            Some(profile_id),
            Some(hardware),
            Some(os_version),
            Some(topology),
            Some(source_commit),
            Some(source_tree),
            Some(runner_sha256),
            Some(context_sha256),
        ] = values
        else {
            return Err("resource_arguments_incomplete");
        };
        if values.into_iter().flatten().any(|value| {
            value.trim().is_empty()
                || value.len() > METADATA_LIMIT
                || !value.bytes().all(|byte| (b' '..=b'~').contains(&byte))
        }) {
            return Err("resource_metadata_invalid");
        }
        for (value, length) in [
            (source_commit, 40),
            (source_tree, 40),
            (runner_sha256, 64),
            (context_sha256, 64),
        ] {
            if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("resource_digest_invalid");
            }
        }
        Ok(Some(Self {
            profile_id: profile_id.to_owned(),
            hardware: hardware.to_owned(),
            os_version: os_version.to_owned(),
            topology: topology.to_owned(),
            source_commit: source_commit.to_owned(),
            source_tree: source_tree.to_owned(),
            runner_sha256: runner_sha256.to_owned(),
            context_sha256: context_sha256.to_owned(),
        }))
    }
}

#[derive(Clone, Copy)]
pub(super) struct ResourceArtifacts<'a> {
    pub(super) consumer: &'a str,
    pub(super) library: &'a str,
    pub(super) fixture: &'a str,
    pub(super) runner: &'a str,
}

impl<'a> ResourceArtifacts<'a> {
    pub(super) fn from_artifacts(artifacts: &'a [Artifact]) -> Option<Self> {
        let [consumer, library, fixture, runner] = artifacts else {
            return None;
        };
        if [consumer.role, library.role, fixture.role, runner.role]
            != ["consumer", "library", "fixture", "runner"]
        {
            return None;
        }
        Some(Self {
            consumer: &consumer.before.sha256,
            library: &library.before.sha256,
            fixture: &fixture.before.sha256,
            runner: &runner.before.sha256,
        })
    }
}

pub(super) struct ResourceBudget {
    pub(super) name: &'static str,
    pub(super) unit: &'static str,
    pub(super) absolute_lifecycle: u64,
    pub(super) delta_lifecycle: u64,
    pub(super) absolute_final: u64,
    pub(super) delta_final: u64,
}

pub(super) struct ProfileGeometry {
    pub(super) frame: [u64; 2],
    pub(super) transform: [f64; 4],
}

pub(super) struct ResourceProfile {
    pub(super) id: &'static str,
    pub(super) target: &'static str,
    pub(super) hardware: &'static str,
    pub(super) os_version: &'static str,
    pub(super) topology: &'static str,
    document: &'static str,
    pub(super) document_sha256: &'static str,
    pub(super) consumer_sha256: [&'static str; 2],
    pub(super) library_sha256: &'static str,
    pub(super) fixture_sha256: &'static str,
    pub(super) budgets: [ResourceBudget; 3],
    pub(super) geometry: [ProfileGeometry; 3],
}

// As with template_watch_boundary/budgets.rs, numeric limits are compiled;
// include_str! plus the reviewed document byte pin rejects profile drift.
pub(super) const PROFILES: [ResourceProfile; 2] = [
    ResourceProfile {
        id: "phase-5-native-foreign-resources-aarch64-apple-darwin",
        target: "aarch64-apple-darwin",
        hardware: "Apple M1 Pro",
        os_version: "macOS 26.6.2 (25G83)",
        topology: "Retina2x-to-external1x;1280x904->1376x968->688x484",
        document: include_str!(
            "../../../../docs/benchmarks/phase-5-native-foreign-resources-aarch64-apple-darwin.toml"
        ),
        document_sha256: "56df926561320e76de4a1930f54f1cbc89cdc6cbd57f409dbfd2c252a338b7ad",
        consumer_sha256: [
            "6117435fc29e8bcf2a4937931af10d6b7a22daf96e98a5b50a8f675532fe8930",
            "b3f45b4d5df20228662fc749ced25b9880a97120292233f61f96752693498a43",
        ],
        library_sha256: "41d2af7a6d0cba8d55d0ab552a53dabe88826b7fe50c4bd18fe26cb314893c63",
        fixture_sha256: "d7fe848cccaf2db829dafd538b36460d1c015d33d666ac4158e015d4e700e307",
        budgets: [
            ResourceBudget {
                name: "physical_footprint_bytes",
                unit: "bytes",
                absolute_lifecycle: 151_945_216,
                delta_lifecycle: 2_646_016,
                absolute_final: 555_094_016,
                delta_final: 405_917_696,
            },
            ResourceBudget {
                name: "resident_bytes",
                unit: "bytes",
                absolute_lifecycle: 219_361_280,
                delta_lifecycle: 2_703_360,
                absolute_final: 622_305_280,
                delta_final: 405_954_560,
            },
            ResourceBudget {
                name: "mach_port_name_count",
                unit: "count",
                absolute_lifecycle: 270,
                delta_lifecycle: 8,
                absolute_final: 275,
                delta_final: 13,
            },
        ],
        geometry: [
            ProfileGeometry {
                frame: [1280, 904],
                transform: [640.0, 452.0, 2.0, 2.0],
            },
            ProfileGeometry {
                frame: [1376, 968],
                transform: [688.0, 484.0, 2.0, 2.0],
            },
            ProfileGeometry {
                frame: [688, 484],
                transform: [688.0, 484.0, 1.0, 1.0],
            },
        ],
    },
    ResourceProfile {
        id: "phase-5-native-foreign-resources-x86_64-pc-windows-msvc",
        target: "x86_64-pc-windows-msvc",
        hardware: "Intel Core i7-12700KF; NVIDIA RTX 4080; driver32.0.15.9186",
        os_version: "Windows 11 Pro 25H2 (26200.9278)",
        topology: "primary3840x2160@150%;left3840x2160@125%;342x231->462x311->464x312",
        document: include_str!(
            "../../../../docs/benchmarks/phase-5-native-foreign-resources-x86_64-pc-windows-msvc.toml"
        ),
        document_sha256: "f3e7da4a3663d902d67d29c99ea9d8858a1aa39650e2a89d77d8fa03a625463d",
        consumer_sha256: [
            "2d8040c9804263e6ebd5f8397d6d5c2e8f9681034f30e08b5d683a418fe19248",
            "e8e1d5ae6b1ac263f9530b8e3dca44d6be28c22c4f79eb27c095a8cc755e9900",
        ],
        library_sha256: "c95dfd07bf1ee9da09ddd467cf62eb339499b5630c90610d1d3c6ed45bf28ac1",
        fixture_sha256: "f7c0a718db6e9b7a571a7422e298ad104b8df1c9cefa259dd6e0e9088a5408d8",
        budgets: [
            ResourceBudget {
                name: "private_commit_bytes",
                unit: "bytes",
                absolute_lifecycle: 35_532_800,
                delta_lifecycle: 8_712_192,
                absolute_final: 54_779_904,
                delta_final: 26_951_680,
            },
            ResourceBudget {
                name: "working_set_bytes",
                unit: "bytes",
                absolute_lifecycle: 49_446_912,
                delta_lifecycle: 4_513_792,
                absolute_final: 55_513_088,
                delta_final: 10_342_400,
            },
            ResourceBudget {
                name: "process_handle_count",
                unit: "count",
                absolute_lifecycle: 534,
                delta_lifecycle: 53,
                absolute_final: 539,
                delta_final: 58,
            },
        ],
        geometry: [
            ProfileGeometry {
                frame: [342, 231],
                transform: [342.0, 231.0, 1.0, 1.0],
            },
            ProfileGeometry {
                frame: [462, 311],
                transform: [462.0, 311.0, 1.0, 1.0],
            },
            ProfileGeometry {
                frame: [464, 312],
                transform: [465.2608695652174, 312.0, 0.997289972899729, 1.0],
            },
        ],
    },
];

impl ResourceProfile {
    pub(super) fn admit(
        &self,
        request: &ResourceRequest,
        target: &str,
        release: bool,
        document_sha256: &str,
        artifacts: Option<ResourceArtifacts<'_>>,
    ) -> Checked<()> {
        if self.target != target || !release {
            return Err("resource_build_inapplicable");
        }
        if document_sha256 != self.document_sha256 {
            return Err("resource_profile_document_changed");
        }
        if request.hardware != self.hardware
            || request.os_version != self.os_version
            || request.topology != self.topology
        {
            return Err("resource_declared_profile_mismatch");
        }
        let artifacts = artifacts.ok_or("resource_artifacts_missing")?;
        if !self.consumer_sha256.contains(&artifacts.consumer)
            || artifacts.library != self.library_sha256
            || artifacts.fixture != self.fixture_sha256
        {
            return Err("resource_artifact_inapplicable");
        }
        if !artifacts
            .runner
            .eq_ignore_ascii_case(&request.runner_sha256)
        {
            return Err("resource_runner_mismatch");
        }
        Ok(())
    }
}

pub(super) struct ResourceAdmission<'a> {
    request: Option<&'a ResourceRequest>,
    pub(super) profile: Option<&'static ResourceProfile>,
    document_sha256: Option<String>,
    pub(super) error: Option<Failure>,
}

impl<'a> ResourceAdmission<'a> {
    pub(super) fn select(
        request: Option<&'a ResourceRequest>,
        target: &str,
        release: bool,
        artifacts: Option<ResourceArtifacts<'_>>,
    ) -> Self {
        let profile = request.and_then(|request| {
            PROFILES
                .iter()
                .find(|profile| profile.id == request.profile_id)
        });
        let document_sha256 = profile.map(|profile| {
            super::native_watch_report::qualification_bytes_sha256(profile.document.as_bytes())
        });
        let error = request.and_then(|request| match (profile, document_sha256.as_deref()) {
            (Some(profile), Some(digest)) => profile
                .admit(request, target, release, digest, artifacts)
                .err(),
            _ => Some("resource_profile_unknown"),
        });
        Self {
            request,
            profile,
            document_sha256,
            error,
        }
    }

    pub(super) fn append_json(&self, text: &mut String) {
        let _ = write!(
            text,
            ",\"resource_qualification\":{{\"enforcement_requested\":{},\"prelaunch_admitted\":{},\"scope\":\"sampled post-release/pre-exit process private-or-footprint bytes, resident bytes and handle-or-port counts\",\"excludes\":\"peak, GPU bytes, exact live-object counts, plateau, no-growth, leak-free, cache attribution\",\"physical_host_authenticated\":false,\"source_inventory_authenticated\":false,\"external_frozen_campaign_review\":\"required_separately\",\"context_digest_scope\":\"caller-declared reference, not approval or authentication\",\"native_support_promoted\":false,\"metadata_limit_bytes\":{METADATA_LIMIT},\"expected_samples\":5,\"expected_comparisons_per_consumer\":{COMPARISON_COUNT},\"outer_warmup_enforced_when_selected\":true,\"consumer_baseline\":\"own F1 (0,0), never rebased or pooled\",\"error\":",
            self.request.is_some(),
            self.request.is_some() && self.error.is_none(),
        );
        append_optional_string(text, self.error);
        text.push_str(",\"declarations\":");
        if let Some(request) = self.request {
            text.push('{');
            for (index, (key, value)) in [
                ("profile", request.profile_id.as_str()),
                ("hardware", request.hardware.as_str()),
                ("os_version", request.os_version.as_str()),
                ("topology", request.topology.as_str()),
                ("source_commit", request.source_commit.as_str()),
                ("source_tree", request.source_tree.as_str()),
                ("runner_sha256", request.runner_sha256.as_str()),
                ("context_sha256", request.context_sha256.as_str()),
            ]
            .into_iter()
            .enumerate()
            {
                if index != 0 {
                    text.push(',');
                }
                let _ = write!(text, "\"{key}\":");
                append_string(text, value);
            }
            text.push('}');
        } else {
            text.push_str("null");
        }
        text.push_str(",\"compiled_profile\":");
        if let Some(profile) = self.profile {
            let _ = write!(
                text,
                "{{\"id\":\"{}\",\"target\":\"{}\",\"accepted_limits\":true,\"sha256\":\"{}\",\"observed_document_sha256\":",
                profile.id, profile.target, profile.document_sha256,
            );
            append_optional_string(text, self.document_sha256.as_deref());
            text.push_str(",\"metrics\":[");
            for (index, budget) in profile.budgets.iter().enumerate() {
                if index != 0 {
                    text.push(',');
                }
                let _ = write!(
                    text,
                    "{{\"index\":{index},\"name\":\"{}\",\"unit\":\"{}\",\"absolute_lifecycle\":{},\"delta_lifecycle\":{},\"absolute_final\":{},\"delta_final\":{}}}",
                    budget.name,
                    budget.unit,
                    budget.absolute_lifecycle,
                    budget.delta_lifecycle,
                    budget.absolute_final,
                    budget.delta_final,
                );
            }
            text.push_str("]}");
        } else {
            text.push_str("null");
        }
        text.push('}');
    }
}

// The CLI admits printable ASCII only; escaping still matters for rejected
// caller declarations, which must be preserved without corrupting the report.
fn append_string(text: &mut String, value: &str) {
    text.push('"');
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            text.push('\\');
        }
        text.push(character);
    }
    text.push('"');
}

fn append_optional_string(text: &mut String, value: Option<&str>) {
    if let Some(value) = value {
        append_string(text, value);
    } else {
        text.push_str("null");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResourceStage {
    Baseline,
    LifecycleOne,
    LifecycleTwo,
    LifecycleThree,
    FinalRelease,
}

impl ResourceStage {
    const ALL: [Self; 5] = [
        Self::Baseline,
        Self::LifecycleOne,
        Self::LifecycleTwo,
        Self::LifecycleThree,
        Self::FinalRelease,
    ];

    const fn wire(self) -> (u64, u64) {
        match self {
            Self::Baseline => (0, 0),
            Self::LifecycleOne => (1, 1),
            Self::LifecycleTwo => (1, 2),
            Self::LifecycleThree => (1, 3),
            Self::FinalRelease => (2, 4),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResourceSample {
    pub(super) stage: ResourceStage,
    pub(super) values: [u64; 3],
}

pub(super) fn validate_sample(row: usize, index: usize, fact: &Fact) -> Checked<ResourceSample> {
    let stage = match (row, index) {
        (0, 0..=3) => ResourceStage::ALL[index],
        (8, 0) => ResourceStage::FinalRelease,
        _ => return Err("resource_stage_invalid"),
    };
    let [
        Number::Unsigned(phase),
        Number::Unsigned(cycle),
        Number::Unsigned(first),
        Number::Unsigned(resident),
        Number::Unsigned(count),
    ] = fact.values.as_slice()
    else {
        return Err("resource_sample_invalid");
    };
    if fact.key != FactKey::Resource || (*phase, *cycle) != stage.wire() {
        return Err("resource_stage_invalid");
    }
    if *first > 1 << 50 || *resident > 1 << 50 || *count > 16_777_216 {
        return Err("resource_sample_invalid");
    }
    Ok(ResourceSample {
        stage,
        values: [*first, *resident, *count],
    })
}

pub(super) struct ResourceSamples(pub(super) [ResourceSample; 5]);

impl ResourceSamples {
    fn from_ledger(ledger: &Ledger) -> Checked<Self> {
        let mut samples = [None; 5];
        for (row_index, row) in ledger.rows.iter().enumerate() {
            let mut resource_index = 0;
            let mut bootstraps = 0;
            for fact in &row.facts {
                if fact.key == FactKey::Bootstrap {
                    bootstraps += 1;
                }
                if fact.key != FactKey::Resource {
                    continue;
                }
                let sample = validate_sample(row_index, resource_index, fact)?;
                if row_index == 0 && bootstraps != resource_index + 1 {
                    return Err("resource_bootstrap_order");
                }
                let index = if row_index == 0 { resource_index } else { 4 };
                samples[index] = Some(sample);
                resource_index += 1;
            }
        }
        match samples {
            [
                Some(baseline),
                Some(one),
                Some(two),
                Some(three),
                Some(final_release),
            ] => Ok(Self([baseline, one, two, three, final_release])),
            _ => Err("resource_sample_missing"),
        }
    }

    fn comparisons(&self, profile: &ResourceProfile) -> [ResourceComparison; COMPARISON_COUNT] {
        std::array::from_fn(|index| {
            // 3 baseline absolute + (3 lifecycle + 1 final) * 3 metrics * 2 gates.
            let (sample, metric, delta) = if index < 3 {
                (0, index, false)
            } else {
                (
                    1 + (index - 3) / 6,
                    ((index - 3) % 6) / 2,
                    (index - 3) % 2 == 1,
                )
            };
            let budget = &profile.budgets[metric];
            let limit = match (sample == 4, delta) {
                (false, false) => budget.absolute_lifecycle,
                (false, true) => budget.delta_lifecycle,
                (true, false) => budget.absolute_final,
                (true, true) => budget.delta_final,
            };
            let observed = i128::from(self.0[sample].values[metric]);
            ResourceComparison {
                sample,
                metric,
                delta,
                observed: if delta {
                    observed - i128::from(self.0[0].values[metric])
                } else {
                    observed
                },
                limit,
            }
        })
    }
}

pub(super) struct ResourceComparison {
    pub(super) sample: usize,
    pub(super) metric: usize,
    pub(super) delta: bool,
    pub(super) observed: i128,
    pub(super) limit: u64,
}

impl ResourceComparison {
    pub(super) fn passed(&self) -> bool {
        self.observed <= i128::from(self.limit)
    }

    fn append_json(&self, text: &mut String) {
        let _ = write!(
            text,
            "{{\"sample\":{},\"metric\":{},\"kind\":\"{}\",\"observed\":{},\"limit\":{},\"passed\":{}}}",
            self.sample,
            self.metric,
            if self.delta {
                "signed_delta"
            } else {
                "absolute"
            },
            self.observed,
            self.limit,
            self.passed(),
        );
    }
}

#[derive(Clone, Copy, PartialEq)]
enum GeometryScalars {
    Frame([u64; 2]),
    Transform([f64; 4]),
}

impl GeometryScalars {
    fn from_fact(fact: &Fact) -> Option<Self> {
        match (fact.key, fact.values.as_slice()) {
            (
                FactKey::Frame,
                [
                    _,
                    _,
                    _,
                    _,
                    Number::Unsigned(width),
                    Number::Unsigned(height),
                ],
            ) => Some(Self::Frame([*width, *height])),
            (
                FactKey::Transform,
                [
                    _,
                    _,
                    _,
                    Number::Real(width),
                    Number::Real(height),
                    Number::Real(x),
                    Number::Real(y),
                    _,
                ],
            ) if [*width, *height, *x, *y].into_iter().all(f64::is_finite) => {
                Some(Self::Transform([*width, *height, *x, *y]))
            }
            _ => None,
        }
    }

    fn expected(profile: &ProfileGeometry, key: FactKey) -> Self {
        if key == FactKey::Frame {
            Self::Frame(profile.frame)
        } else {
            Self::Transform(profile.transform)
        }
    }

    fn append_json(self, text: &mut String) {
        match self {
            Self::Frame([width, height]) => {
                let _ = write!(text, "[{width},{height}]");
            }
            Self::Transform([width, height, x, y]) => {
                let _ = write!(text, "[{width},{height},{x},{y}]");
            }
        }
    }
}

struct GeometryViolation {
    row: usize,
    key: FactKey,
    occurrence: usize,
    reason: Failure,
    before: GeometryScalars,
    after: GeometryScalars,
    observed: Option<GeometryScalars>,
}

impl GeometryViolation {
    fn append_json(&self, text: &mut String) {
        let _ = write!(
            text,
            "{{\"row\":\"F{}\",\"key\":\"{}\",\"occurrence\":{},\"reason\":\"{}\",\"expected_before\":",
            self.row + 1,
            self.key.name(),
            self.occurrence,
            self.reason,
        );
        self.before.append_json(text);
        text.push_str(",\"expected_after\":");
        self.after.append_json(text);
        text.push_str(",\"observed\":");
        if let Some(observed) = self.observed {
            observed.append_json(text);
        } else {
            text.push_str("null");
        }
        text.push('}');
    }
}

fn geometry_violation(profile: &ResourceProfile, ledger: &Ledger) -> Option<GeometryViolation> {
    for (row, before, after) in [(1, 0, 0), (2, 0, 0), (3, 0, 1), (4, 1, 2)] {
        for key in [FactKey::Frame, FactKey::Transform] {
            let before = GeometryScalars::expected(&profile.geometry[before], key);
            let after = GeometryScalars::expected(&profile.geometry[after], key);
            let mut seen_before = false;
            let mut seen_after = false;
            let mut count = 0;
            // C and C++ interleave these keys differently and may repeat frames.
            // Validate each key's entire monotonic before/after transition, not
            // incidental counts, target ordinals, source stamps or desktop origins.
            for fact in ledger.rows[row].facts.iter().filter(|fact| fact.key == key) {
                let observed = GeometryScalars::from_fact(fact);
                if observed == Some(before) && !seen_after {
                    seen_before = true;
                } else if observed == Some(after) && seen_before {
                    seen_after = true;
                } else {
                    return Some(GeometryViolation {
                        row,
                        key,
                        occurrence: count,
                        reason: "resource_geometry_mismatch",
                        before,
                        after,
                        observed,
                    });
                }
                count += 1;
            }
            if !seen_before || (before != after && !seen_after) {
                return Some(GeometryViolation {
                    row,
                    key,
                    occurrence: count,
                    reason: "resource_geometry_missing",
                    before,
                    after,
                    observed: None,
                });
            }
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResourceState {
    Pending,
    Unaccepted,
    PrerequisiteUnavailable,
    Passed,
    Exceeded,
    Rejected(Failure),
}

pub(super) struct ResourceDecision {
    state: ResourceState,
    pub(super) samples: Option<ResourceSamples>,
    pub(super) comparisons: Option<[ResourceComparison; COMPARISON_COUNT]>,
    geometry_applicable: Option<bool>,
    geometry_violation: Option<GeometryViolation>,
    effective_outcomes: [Outcome; 9],
    aggregate: Outcome,
    eligible: bool,
}

impl ResourceDecision {
    pub(super) fn evaluate(
        admission: &ResourceAdmission<'_>,
        ledger: &Ledger,
        phase: Phase,
        infra: bool,
    ) -> Self {
        let mut decision = Self::observe(admission, ledger, phase);
        let infra = infra || admission.error.is_some() || (phase == Phase::Final && !ledger.done);
        decision.effective_outcomes = std::array::from_fn(|row| {
            let consumer = ledger.rows[row].outcome.unwrap_or(Outcome::Infra);
            let observed = if row == 8 {
                consumer.combine(decision.outcome())
            } else {
                consumer
            };
            phase.aggregate(observed, infra)
        });
        decision.aggregate = decision
            .effective_outcomes
            .iter()
            .copied()
            .fold(Outcome::Pass, Outcome::combine);
        // Only the unselected, structurally complete precursor may advance
        // despite UNEXECUTED. No consumer or cleanup failure is rescued.
        decision.eligible = phase == Phase::Final
            && !infra
            && decision.may_advance()
            && ledger.loaded
            && ledger.done
            && ledger.next == ledger.rows.len()
            && ledger
                .rows
                .iter()
                .all(|row| row.outcome == Some(Outcome::Pass));
        decision
    }

    fn observe(admission: &ResourceAdmission<'_>, ledger: &Ledger, phase: Phase) -> Self {
        let mut decision = Self {
            state: ResourceState::Pending,
            samples: None,
            comparisons: None,
            geometry_applicable: None,
            geometry_violation: None,
            effective_outcomes: [Outcome::Unexecuted; 9],
            aggregate: Outcome::Unexecuted,
            eligible: false,
        };
        if let Some(error) = admission.error {
            decision.state = ResourceState::Rejected(error);
            return decision;
        }
        if phase == Phase::Before {
            return decision;
        }
        let samples = match ResourceSamples::from_ledger(ledger) {
            Ok(samples) => samples,
            Err("resource_sample_missing")
                if ledger.rows[0].outcome != Some(Outcome::Pass)
                    || ledger.rows[8].outcome != Some(Outcome::Pass) =>
            {
                // Failed/unavailable consumer work is not a zero measurement.
                // Preserve its outcome; a claimed successful observation must
                // instead supply every sample, even without a selected profile.
                decision.state = ResourceState::PrerequisiteUnavailable;
                return decision;
            }
            Err(error) => {
                decision.state = ResourceState::Rejected(error);
                return decision;
            }
        };
        if let Some(profile) = admission.profile {
            let comparisons = samples.comparisons(profile);
            decision.geometry_violation = geometry_violation(profile, ledger);
            decision.geometry_applicable = Some(decision.geometry_violation.is_none());
            decision.state = if let Some(violation) = &decision.geometry_violation {
                ResourceState::Rejected(violation.reason)
            } else if comparisons.iter().all(ResourceComparison::passed) {
                ResourceState::Passed
            } else {
                ResourceState::Exceeded
            };
            decision.comparisons = Some(comparisons);
        } else {
            decision.state = ResourceState::Unaccepted;
        }
        decision.samples = Some(samples);
        decision
    }

    pub(super) fn outcome(&self) -> Outcome {
        match self.state {
            ResourceState::Passed => Outcome::Pass,
            ResourceState::Exceeded => Outcome::Fail,
            ResourceState::Rejected(_) => Outcome::Infra,
            _ => Outcome::Unexecuted,
        }
    }

    pub(super) fn reason(&self) -> Failure {
        match self.state {
            ResourceState::Pending => "resource_observation_not_started",
            ResourceState::Unaccepted => "resource_budget_unaccepted",
            ResourceState::PrerequisiteUnavailable => "resource_consumer_prerequisite_unavailable",
            ResourceState::Passed => "resource_budget_passed",
            ResourceState::Exceeded => "resource_budget_exceeded",
            ResourceState::Rejected(error) => error,
        }
    }

    pub(super) fn may_advance(&self) -> bool {
        matches!(
            self.state,
            ResourceState::Unaccepted | ResourceState::Passed
        )
    }

    pub(super) fn effective(&self, row: usize) -> Outcome {
        self.effective_outcomes[row]
    }

    pub(super) fn aggregate(&self) -> Outcome {
        self.aggregate
    }

    pub(super) fn eligible(&self) -> bool {
        self.eligible
    }

    pub(super) fn mark_infrastructure_failure(&mut self) {
        self.effective_outcomes.fill(Outcome::Infra);
        self.aggregate = Outcome::Infra;
        self.eligible = false;
    }

    pub(super) fn append_json(&self, text: &mut String) {
        let _ = write!(
            text,
            "{{\"outcome\":\"{}\",\"reason\":\"{}\",\"numeric_pass\":{},\"effective_f9\":\"{}\",\"aggregate\":\"{}\",\"eligible\":{},\"may_advance_resource_gate\":{},\"geometry_applicable\":{},\"geometry_violation_limit\":1,\"geometry_violation\":",
            self.outcome().name(),
            self.reason(),
            self.state == ResourceState::Passed,
            self.effective(8).name(),
            self.aggregate().name(),
            self.eligible(),
            self.may_advance(),
            super::option_json(self.geometry_applicable),
        );
        if let Some(violation) = &self.geometry_violation {
            violation.append_json(text);
        } else {
            text.push_str("null");
        }
        text.push_str(",\"samples\":");
        if let Some(samples) = &self.samples {
            text.push('[');
            for (index, sample) in samples.0.iter().enumerate() {
                if index != 0 {
                    text.push(',');
                }
                let (phase, cycle) = sample.stage.wire();
                let [first, resident, count] = sample.values;
                let _ = write!(
                    text,
                    "{{\"phase\":{phase},\"cycle\":{cycle},\"values\":[{first},{resident},{count}]}}"
                );
            }
            text.push(']');
        } else {
            text.push_str("null");
        }
        let _ = write!(
            text,
            ",\"comparison_count\":{},\"comparisons\":[",
            self.comparisons
                .as_ref()
                .map_or(0, |comparisons| comparisons.len()),
        );
        if let Some(comparisons) = &self.comparisons {
            for (index, comparison) in comparisons.iter().enumerate() {
                if index != 0 {
                    text.push(',');
                }
                comparison.append_json(text);
            }
        }
        let _ = write!(
            text,
            "],\"violation_limit\":{COMPARISON_COUNT},\"violations\":["
        );
        if let Some(comparisons) = &self.comparisons {
            for (index, comparison) in comparisons
                .iter()
                .filter(|comparison| !comparison.passed())
                .enumerate()
            {
                if index != 0 {
                    text.push(',');
                }
                comparison.append_json(text);
            }
        }
        text.push_str("]}");
    }
}
