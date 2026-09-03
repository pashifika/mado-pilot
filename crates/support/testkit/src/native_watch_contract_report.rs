//! Versioned, content-redacted result schema for the required native watcher contract.
//!
//! This module is deliberately separate from [`crate::native_watch_report`]. The
//! latter owns immutable V1 statistical evidence; this module owns the compact
//! one-shot Lane B semantic result.

use serde::{Deserialize, Serialize};

/// Exact schema identity for compact native template-watch contract results.
pub const SCHEMA_VERSION: &str = "mado-pilot.native-template-watch-contract.v2";

/// Durable qualification lane carried by this schema.
pub const LANE: &str = "B";

const MAX_REPORT_BYTES: usize = 64 * 1024;

/// Stable scenario order required for a complete Lane B execution.
pub const SCENARIOS: [ScenarioName; 8] = [
    ScenarioName::Admission,
    ScenarioName::PostOpenTokenSynchronization,
    ScenarioName::WatcherMatchCorrelation,
    ScenarioName::TwoSessionFairness,
    ScenarioName::GeometryGeneration,
    ScenarioName::RetainedOwnershipAndFreshSession,
    ScenarioName::LifecycleTermination,
    ScenarioName::CleanupBaseline,
];

/// Approved native host represented without host names or other environment data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePlatform {
    /// `x86_64-pc-windows-msvc` native WGC execution.
    WindowsX86_64,
    /// `aarch64-apple-darwin` native ScreenCaptureKit execution.
    MacosAarch64,
}

/// Outer execution classification. Only `Fail` means product semantics executed and failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionOutcome {
    /// Every required semantic and cleanup fact passed.
    Pass,
    /// Product contract execution produced a semantic or cleanup failure.
    Fail,
    /// Apparatus authority was unavailable before the product contract could execute.
    Infra,
    /// The host lacks a declared prerequisite for this native contract.
    Unsupported,
}

/// Stable name of one compact native integration scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioName {
    /// Target discovery and non-prompting permission admission.
    Admission,
    /// Exact post-open absent-token observation.
    PostOpenTokenSynchronization,
    /// Absent-to-visible match correlated to captured token pixels.
    WatcherMatchCorrelation,
    /// Fair exact-token progress through two native sessions.
    TwoSessionFairness,
    /// Correlation of a commanded visual token with a new geometry generation.
    GeometryGeneration,
    /// Retained native/CPU ownership and causal successor-session progress.
    RetainedOwnershipAndFreshSession,
    /// Exact target, session, and scheduler terminal outcomes.
    LifecycleTermination,
    /// Explicit platform fixture and native resource baseline.
    CleanupBaseline,
}

/// Stable stage at which execution stopped. It never contains backend detail text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureStage {
    /// Command-line protocol or result-schema admission.
    Protocol,
    /// Private fixture launch and authentication.
    FixtureLaunch,
    /// Native engine construction.
    EngineCreate,
    /// Non-prompting capture permission observation.
    PermissionAdmission,
    /// Exact fixture target discovery.
    TargetDiscovery,
    /// Native session open.
    SessionOpen,
    /// Post-open exact-token synchronization.
    Readiness,
    /// Template preparation from an authoritative frame.
    TemplatePreparation,
    /// Watch admission.
    WatchStart,
    /// Proof that pending work consumed the absent state.
    WatchProgress,
    /// Acknowledged visible-token stimulus.
    VisualStimulus,
    /// Terminal result and source-frame correlation.
    MatchCorrelation,
    /// Fair two-session progress.
    TwoSessionProgress,
    /// Commanded geometry transition and frame authority.
    GeometryTransition,
    /// Retained result, frame, or CPU mapping behavior.
    RetainedOwnership,
    /// Successor native session progress.
    FreshSession,
    /// Target-loss terminal authority.
    TargetTermination,
    /// Session-close terminal authority.
    SessionTermination,
    /// Engine/scheduler-close terminal authority.
    EngineTermination,
    /// Native session or engine resource release.
    NativeCleanup,
    /// Explicit private fixture finalization.
    FixtureFinalize,
    /// Declared platform resource baseline.
    ResourceBaseline,
}

/// Bounded failure class. Free-form platform payloads cannot enter the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The runner or fixture did not establish authority to execute.
    Authority,
    /// A public operation returned a typed product status.
    Operation,
    /// An exact-token or terminal-state deadline expired.
    Timeout,
    /// An observed result violated the scenario oracle.
    Oracle,
    /// Required resource or fixture cleanup did not reach its baseline.
    Cleanup,
    /// The private runner protocol or schema was internally inconsistent.
    Protocol,
}

/// Stable projection of public native status; unknown future statuses remain bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeStatus {
    /// No public status accompanied this fact.
    None,
    /// Invalid request.
    InvalidArgument,
    /// Unsupported operation or capability.
    Unsupported,
    /// Cancellation won.
    Cancelled,
    /// Absolute deadline expired.
    DeadlineExceeded,
    /// Handle or session was closed.
    Closed,
    /// Target lifetime ended.
    TargetLost,
    /// A documented resource bound was reached.
    LimitExceeded,
    /// Capture responsibility failed.
    CaptureFailed,
    /// Asset admission failed.
    AssetInvalid,
    /// Vision responsibility failed.
    VisionFailed,
    /// Input responsibility failed.
    InputFailed,
    /// Internal invariant failed.
    Internal,
    /// A future non-exhaustive status not known to this schema revision.
    Other,
}

/// Bounded lifecycle observation attached to a failure or success diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleFact {
    /// No product handle had been admitted.
    NotStarted,
    /// The relevant native session was open.
    Open,
    /// The session was explicitly closed.
    SessionClosed,
    /// The capture target was lost.
    TargetLost,
    /// The engine scheduler was closed.
    SchedulerClosed,
    /// All owned native resources reached the declared baseline.
    Released,
}

/// One complete frame identity represented only by bounded numeric ordinals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameIdentityFacts {
    /// Stream ordinal.
    pub stream: u64,
    /// Stream epoch.
    pub epoch: u64,
    /// Sequence within the epoch.
    pub sequence: u64,
    /// Authoritative geometry revision.
    pub geometry: u64,
}

/// Failure payload shared by semantic, cleanup, and apparatus results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureFacts {
    /// Stable failure class.
    pub kind: FailureKind,
    /// Stable execution stage.
    pub stage: FailureStage,
    /// Bounded public status projection.
    pub status: NativeStatus,
}

/// Independent semantic or cleanup result for one scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum ScenarioFact {
    /// The fact passed.
    Pass,
    /// The fact failed with a bounded typed payload.
    Fail {
        /// Typed failure payload.
        failure: FailureFacts,
    },
}

impl ScenarioFact {
    /// Builds a failed fact without permitting free-form diagnostic text.
    #[must_use]
    pub const fn failed(kind: FailureKind, stage: FailureStage, status: NativeStatus) -> Self {
        Self::Fail {
            failure: FailureFacts {
                kind,
                stage,
                status,
            },
        }
    }

    /// Returns whether the fact passed.
    #[must_use]
    pub const fn is_pass(self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// Non-overlapping monotonic timing intervals for one scenario.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioTiming {
    /// Accepted open through exact readiness-token observation.
    pub startup_micros: Option<u64>,
    /// Visible-token acknowledgement through correlated terminal outcome.
    pub watch_micros: Option<u64>,
    /// Close/finalize request through the declared resource baseline.
    pub teardown_micros: Option<u64>,
}

/// Exact-lifetime state projected without a process identifier or native object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessLifetimeFact {
    /// The lifetime was not queried because an earlier authoritative process remained live.
    NotObserved,
    /// No accepted workspace observation exists yet.
    Unknown,
    /// The exact retained launch is live.
    Live,
    /// The exact retained launch is gone.
    Lost,
    /// The exact-lifetime probe failed.
    ObservationFailed,
}

/// Bounded exact-lifetime cleanup debt after fixture ownership release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupDebtFact {
    /// No deferred exact-lifetime cleanup remains.
    None,
    /// A bounded reaper owns remaining cleanup.
    Deferred,
}

/// Platform-specific finalization counters reduced to non-sensitive bounded facts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFacts {
    /// The exact private fixture process reached its terminal baseline.
    pub fixture_process_reaped: bool,
    /// The bounded fixture output reader joined cleanly.
    pub fixture_reader_joined: bool,
    /// Platform control protocol acknowledged an explicit stop, when applicable.
    pub protocol_stop_acknowledged: Option<bool>,
    /// Authenticated peer process lifetime observed during finalization.
    pub authenticated_lifetime: Option<ProcessLifetimeFact>,
    /// Retained launched application lifetime observed during finalization.
    pub launched_lifetime: Option<ProcessLifetimeFact>,
    /// Containment completed within the predeclared finalization deadline.
    pub bounded_containment: bool,
    /// Apple controller admission observed the exact launched lifetime as `Live`.
    pub apple_launch_accepted_live: Option<bool>,
    /// Protocol events and output were drained without unexpected payload.
    pub output_drained: bool,
    /// The launched executable retained its admitted identity, when applicable.
    pub executable_identity_unchanged: Option<bool>,
    /// Exact-lifetime cleanup debt, when the platform owns that mechanism.
    pub cleanup_debt: Option<CleanupDebtFact>,
    /// macOS exact-lifetime cleanup records scheduled during this scenario.
    pub apple_cleanup_scheduled: Option<u64>,
    /// macOS exact-lifetime cleanup records active at the final baseline.
    pub apple_cleanup_active: Option<u64>,
    /// macOS exact-lifetime cleanup records completed during this scenario.
    pub apple_cleanup_completed: Option<u64>,
    /// macOS exact-lifetime cleanup observations exhausted during this scenario.
    pub apple_cleanup_exhausted: Option<u64>,
}

/// Bounded native publication facts. No pixels, paths, titles, or detail strings fit this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticFacts {
    /// Expected visual token, when this stage used one.
    pub expected_token: Option<u64>,
    /// Last exactly decoded visual token, when one was observed.
    pub observed_token: Option<u64>,
    /// Last public frame identity, when one was observed.
    pub frame: Option<FrameIdentityFacts>,
    /// Target scale before a declared geometry transition, in milli-scale units.
    pub prior_target_scale_milli: Option<[u32; 2]>,
    /// Target scale on the last correlated frame, in milli-scale units.
    pub target_scale_milli: Option<[u32; 2]>,
    /// Relevant lifecycle state.
    pub lifecycle: LifecycleFact,
    /// Bounded frame-acquisition count.
    pub acquisitions: u64,
    /// Bounded publication count.
    pub publications: u64,
    /// Bounded CPU-mapping count.
    pub mappings: u64,
    /// Bounded token-decode count.
    pub decodes: u64,
    /// Last bounded public status.
    pub status: NativeStatus,
    /// Platform-specific finalization counters at the declared baseline.
    pub resources: ResourceFacts,
}

impl Default for DiagnosticFacts {
    fn default() -> Self {
        Self {
            expected_token: None,
            observed_token: None,
            frame: None,
            prior_target_scale_milli: None,
            target_scale_milli: None,
            lifecycle: LifecycleFact::NotStarted,
            acquisitions: 0,
            publications: 0,
            mappings: 0,
            decodes: 0,
            status: NativeStatus::None,
            resources: ResourceFacts::default(),
        }
    }
}

/// Typed result of one compact native integration scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioResult {
    /// Stable scenario identity.
    pub name: ScenarioName,
    /// Product semantic result.
    pub semantic: ScenarioFact,
    /// Cleanup result, independent of semantic success.
    pub cleanup: ScenarioFact,
    /// Non-overlapping scenario timing intervals.
    pub timing: ScenarioTiming,
    /// Content-redacted native diagnostic facts.
    pub diagnostics: DiagnosticFacts,
}

/// Versioned Lane B report. V1 evidence uses a separate module and schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeContractReport {
    schema_version: String,
    lane: String,
    /// Approved native platform.
    pub platform: NativePlatform,
    /// Outer execution classification.
    pub outcome: ExecutionOutcome,
    /// Apparatus failure when product semantics could not execute.
    pub apparatus: Option<FailureFacts>,
    /// Scenario results in [`SCENARIOS`] order.
    pub scenarios: Vec<ScenarioResult>,
}

impl NativeContractReport {
    /// Builds an executed report and derives `PASS` or product `FAIL` from its facts.
    #[must_use]
    pub fn executed(platform: NativePlatform, scenarios: Vec<ScenarioResult>) -> Self {
        let outcome = if scenarios.len() == SCENARIOS.len()
            && scenarios.iter().zip(SCENARIOS).all(|(result, expected)| {
                result.name == expected && result.semantic.is_pass() && result.cleanup.is_pass()
            }) {
            ExecutionOutcome::Pass
        } else {
            ExecutionOutcome::Fail
        };
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            lane: LANE.to_owned(),
            platform,
            outcome,
            apparatus: None,
            scenarios,
        }
    }

    /// Builds an `INFRA` or `UNSUPPORTED` report before product semantics execute.
    #[must_use]
    pub fn not_executed(
        platform: NativePlatform,
        outcome: ExecutionOutcome,
        apparatus: FailureFacts,
        scenarios: Vec<ScenarioResult>,
    ) -> Self {
        assert!(
            matches!(
                outcome,
                ExecutionOutcome::Infra | ExecutionOutcome::Unsupported
            ),
            "only pre-execution classifications are accepted"
        );
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            lane: LANE.to_owned(),
            platform,
            outcome,
            apparatus: Some(apparatus),
            scenarios,
        }
    }

    /// Serializes one canonical compact JSON value followed by no free-form payload.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Parses and validates one V2 report without consulting V1 evidence code.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ReportValidationError> {
        if bytes.len() > MAX_REPORT_BYTES {
            return Err(ReportValidationError::InputTooLarge);
        }
        let report: Self =
            serde_json::from_slice(bytes).map_err(|_| ReportValidationError::MalformedJson)?;
        report.validate()?;
        Ok(report)
    }

    /// Validates schema identity, scenario order, and outcome/fact consistency.
    pub fn validate(&self) -> Result<(), ReportValidationError> {
        if self.schema_version != SCHEMA_VERSION || self.lane != LANE {
            return Err(ReportValidationError::WrongIdentity);
        }
        let ordered_prefix = self
            .scenarios
            .iter()
            .zip(SCENARIOS)
            .all(|(result, expected)| result.name == expected);
        if !ordered_prefix || self.scenarios.len() > SCENARIOS.len() {
            return Err(ReportValidationError::WrongScenarioOrder);
        }
        let all_pass = self.scenarios.len() == SCENARIOS.len()
            && self
                .scenarios
                .iter()
                .all(|result| result.semantic.is_pass() && result.cleanup.is_pass());
        let any_failed = self
            .scenarios
            .iter()
            .any(|result| !result.semantic.is_pass() || !result.cleanup.is_pass());
        let consistent = match self.outcome {
            ExecutionOutcome::Pass => all_pass && self.apparatus.is_none(),
            ExecutionOutcome::Fail => any_failed && self.apparatus.is_none(),
            ExecutionOutcome::Infra | ExecutionOutcome::Unsupported => {
                self.apparatus.is_some() && !any_failed
            }
        };
        if !consistent {
            return Err(ReportValidationError::InconsistentOutcome);
        }
        Ok(())
    }
}

/// Typed V2 report parser failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportValidationError {
    /// Input was not valid JSON for this schema.
    MalformedJson,
    /// Input exceeded the fixed parser allocation boundary.
    InputTooLarge,
    /// Schema or lane identity was not V2 Lane B.
    WrongIdentity,
    /// Required scenarios were duplicated, reordered, or unknown.
    WrongScenarioOrder,
    /// Outer outcome disagreed with apparatus or scenario facts.
    InconsistentOutcome,
}

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticFacts, ExecutionOutcome, FailureFacts, FailureKind, FailureStage,
        MAX_REPORT_BYTES, NativeContractReport, NativePlatform, NativeStatus,
        ReportValidationError, SCENARIOS, ScenarioFact, ScenarioResult, ScenarioTiming,
    };

    fn passing_scenarios() -> Vec<ScenarioResult> {
        SCENARIOS
            .into_iter()
            .map(|name| ScenarioResult {
                name,
                semantic: ScenarioFact::Pass,
                cleanup: ScenarioFact::Pass,
                timing: ScenarioTiming::default(),
                diagnostics: DiagnosticFacts::default(),
            })
            .collect()
    }

    #[test]
    fn complete_ordered_pass_round_trips_as_lane_b_v2() {
        let report =
            NativeContractReport::executed(NativePlatform::MacosAarch64, passing_scenarios());
        let json = report.to_json().expect("the typed report must serialize");
        let parsed = NativeContractReport::from_json(json.as_bytes())
            .expect("the emitted report must validate");

        assert_eq!(parsed, report);
        assert_eq!(parsed.outcome, ExecutionOutcome::Pass);
        assert!(
            json.contains("\"schema_version\":\"mado-pilot.native-template-watch-contract.v2\"")
        );
        assert!(json.contains("\"lane\":\"B\""));
    }

    #[test]
    fn semantic_failure_is_product_fail_without_apparatus_failure() {
        let mut scenarios = passing_scenarios();
        scenarios[2].semantic = ScenarioFact::failed(
            FailureKind::Oracle,
            FailureStage::MatchCorrelation,
            NativeStatus::None,
        );

        let report = NativeContractReport::executed(NativePlatform::WindowsX86_64, scenarios);

        assert_eq!(report.outcome, ExecutionOutcome::Fail);
        assert!(report.apparatus.is_none());
        assert_eq!(report.validate(), Ok(()));
    }

    #[test]
    fn apparatus_failure_never_becomes_product_fail() {
        let failure = FailureFacts {
            kind: FailureKind::Authority,
            stage: FailureStage::PermissionAdmission,
            status: NativeStatus::Unsupported,
        };
        let report = NativeContractReport::not_executed(
            NativePlatform::MacosAarch64,
            ExecutionOutcome::Unsupported,
            failure,
            Vec::new(),
        );

        assert_eq!(report.validate(), Ok(()));
        assert_eq!(report.outcome, ExecutionOutcome::Unsupported);
    }

    #[test]
    fn reordered_or_inconsistent_reports_are_rejected() {
        let mut reordered =
            NativeContractReport::executed(NativePlatform::MacosAarch64, passing_scenarios());
        reordered.scenarios.swap(0, 1);
        assert_eq!(
            reordered.validate(),
            Err(ReportValidationError::WrongScenarioOrder)
        );

        let mut inconsistent =
            NativeContractReport::executed(NativePlatform::MacosAarch64, passing_scenarios());
        inconsistent.outcome = ExecutionOutcome::Fail;
        assert_eq!(
            inconsistent.validate(),
            Err(ReportValidationError::InconsistentOutcome)
        );
    }

    #[test]
    fn schema_cannot_carry_sensitive_or_free_form_fields() {
        let report =
            NativeContractReport::executed(NativePlatform::MacosAarch64, passing_scenarios());
        let json = report.to_json().expect("the typed report must serialize");

        for rejected in [
            "pixels",
            "pixel_hash",
            "target_title",
            "path",
            "credential",
            "process_name",
            "native_payload",
            "detail",
            "message",
        ] {
            assert!(!json.contains(rejected), "unexpected field {rejected}");
        }

        for rejected in [
            "pixels",
            "pixel_hash",
            "target_title",
            "path",
            "credential",
            "unrelated_process",
            "native_payload",
            "detail",
            "message",
        ] {
            let mut injected: serde_json::Value =
                serde_json::from_str(&json).expect("the emitted JSON is valid");
            injected
                .as_object_mut()
                .expect("the report is an object")
                .insert(rejected.to_owned(), serde_json::json!("forbidden"));
            let bytes = serde_json::to_vec(&injected).expect("the injected JSON serializes");
            assert_eq!(
                NativeContractReport::from_json(&bytes),
                Err(ReportValidationError::MalformedJson),
                "unexpectedly admitted field {rejected}"
            );

            let mut nested: serde_json::Value =
                serde_json::from_str(&json).expect("the emitted JSON is valid");
            nested["scenarios"][0]["diagnostics"]
                .as_object_mut()
                .expect("diagnostics is an object")
                .insert(rejected.to_owned(), serde_json::json!("forbidden"));
            let bytes = serde_json::to_vec(&nested).expect("the injected JSON serializes");
            assert_eq!(
                NativeContractReport::from_json(&bytes),
                Err(ReportValidationError::MalformedJson),
                "unexpectedly admitted nested field {rejected}"
            );
        }
    }

    #[test]
    fn oversized_input_is_rejected_before_json_allocation() {
        let bytes = vec![b' '; MAX_REPORT_BYTES + 1];

        assert_eq!(
            NativeContractReport::from_json(&bytes),
            Err(ReportValidationError::InputTooLarge)
        );
    }
}
