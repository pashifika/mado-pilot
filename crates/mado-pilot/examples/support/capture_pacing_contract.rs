//! Closed, payload-free native verification arguments and report.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use mado_pilot::{CapturePacingRequest, OperationContext, Status};
use serde::Serialize;

use super::metrics::SampledMetrics;

pub(super) const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const COOLDOWN: Duration = Duration::from_millis(250);
pub(super) const NATIVE_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const SAMPLE_LIMIT: usize = 1024;
pub(super) const MAPPING_LIMIT: u64 = 8 * 1024 * 1024;
pub(super) const PROCESS_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

pub(super) type Check<T> = Result<T, Failure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    Rule(&'static str),
    NotRun(&'static str),
    Api(Status),
}

impl Failure {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::Rule(reason) | Self::NotRun(reason) => reason,
            Self::Api(Status::Cancelled) => "api-cancelled",
            Self::Api(Status::DeadlineExceeded) => "api-deadline-exceeded",
            Self::Api(Status::Closed) => "api-closed",
            Self::Api(Status::TargetLost) => "api-target-lost",
            Self::Api(Status::Unsupported) => "api-unsupported",
            Self::Api(Status::InvalidArgument) => "api-invalid-argument",
            Self::Api(Status::LimitExceeded) => "api-limit-exceeded",
            Self::Api(Status::CaptureFailed) => "api-capture-failed",
            Self::Api(Status::AssetInvalid) => "api-asset-invalid",
            Self::Api(_) => "api-failed",
        }
    }
}

pub(super) fn api<T>(result: mado_pilot::Result<T>) -> Check<T> {
    result.map_err(|error| Failure::Api(error.status()))
}

pub(super) fn require(condition: bool, reason: &'static str) -> Check<()> {
    if condition {
        Ok(())
    } else {
        Err(Failure::Rule(reason))
    }
}

pub(super) fn nanos(value: Duration) -> Check<u64> {
    u64::try_from(value.as_nanos()).map_err(|_| Failure::Rule("duration-overflow"))
}

pub(super) fn bounded(parent: &OperationContext, limit: Duration) -> Check<OperationContext> {
    let deadline = parent
        .now()
        .checked_add(limit)
        .ok_or(Failure::Rule("deadline-overflow"))?;
    Ok(parent
        .clone()
        .with_deadline(parent.deadline().map_or(deadline, |end| end.min(deadline))))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Case {
    Semantic,
    CaptureOff,
    Baseline,
    CooldownOnly,
    NativeOnly,
    Combined,
}

impl Case {
    fn parse(value: &str) -> Check<Self> {
        match value {
            "semantic" => Ok(Self::Semantic),
            "capture-off" => Ok(Self::CaptureOff),
            "baseline" => Ok(Self::Baseline),
            "cooldown-only" => Ok(Self::CooldownOnly),
            "native-only" => Ok(Self::NativeOnly),
            "combined" => Ok(Self::Combined),
            _ => Err(Failure::Rule("consumer-arguments")),
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::CaptureOff => "capture-off",
            Self::Baseline => "baseline",
            Self::CooldownOnly => "cooldown-only",
            Self::NativeOnly => "native-only",
            Self::Combined => "combined",
        }
    }

    pub(super) const fn cooldown(self) -> Duration {
        match self {
            Self::Semantic | Self::CooldownOnly | Self::Combined => COOLDOWN,
            Self::CaptureOff | Self::Baseline | Self::NativeOnly => Duration::ZERO,
        }
    }

    pub(super) fn pacing(self) -> Check<CapturePacingRequest> {
        match self {
            Self::Semantic | Self::NativeOnly | Self::Combined => {
                api(CapturePacingRequest::required(NATIVE_INTERVAL))
            }
            Self::CaptureOff | Self::Baseline | Self::CooldownOnly => {
                Ok(CapturePacingRequest::source_default())
            }
        }
    }

    pub(super) const fn timeout(self) -> Duration {
        if matches!(self, Self::Semantic) {
            Duration::from_secs(180)
        } else {
            Duration::from_secs(120)
        }
    }
}

#[derive(Debug)]
pub(super) struct Arguments {
    pub(super) case: Case,
    pub(super) fixture: PathBuf,
    pub(super) root: PathBuf,
    pub(super) nonce: String,
    pub(super) token: u64,
    pub(super) image: PathBuf,
    pub(super) model: PathBuf,
    pub(super) runtime: PathBuf,
}

impl Arguments {
    pub(super) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Check<Self> {
        let mut values: [Option<OsString>; 7] = std::array::from_fn(|_| None);
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            let index = match flag.to_str() {
                Some("--case") => 0,
                Some("--fixture") => 1,
                Some("--control-root") => 2,
                Some("--nonce") => 3,
                Some("--image") => 4,
                Some("--model-root") => 5,
                Some("--runtime") => 6,
                _ => return Err(Failure::Rule("consumer-arguments")),
            };
            require(values[index].is_none(), "consumer-arguments")?;
            let value = arguments
                .next()
                .ok_or(Failure::Rule("consumer-arguments"))?;
            require(!value.is_empty(), "consumer-arguments")?;
            values[index] = Some(value);
        }
        let [case, fixture, root, nonce, image, model, runtime] = values;
        let case = case.ok_or(Failure::Rule("consumer-arguments"))?;
        let case = Case::parse(case.to_str().ok_or(Failure::Rule("consumer-arguments"))?)?;
        let nonce = nonce
            .ok_or(Failure::Rule("consumer-arguments"))?
            .into_string()
            .map_err(|_| Failure::Rule("consumer-arguments"))?;
        require(
            nonce.len() == 16
                && nonce
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "consumer-nonce",
        )?;
        let token = u64::from_str_radix(&nonce, 16).map_err(|_| Failure::Rule("consumer-nonce"))?;
        let path = |value: Option<OsString>| {
            value
                .map(PathBuf::from)
                .ok_or(Failure::Rule("consumer-arguments"))
        };
        Ok(Self {
            case,
            fixture: path(fixture)?,
            root: path(root)?,
            nonce,
            token,
            image: path(image)?,
            model: path(model)?,
            runtime: path(runtime)?,
        })
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Pacing {
    pub(super) selection: &'static str,
    pub(super) requested_ns: Option<u64>,
    pub(super) configured_ns: Option<u64>,
    pub(super) outcome: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct Sample {
    pub(super) ocr_ns: u64,
    pub(super) frame_age_ns: Option<u64>,
    pub(super) cooldown_gap_ns: Option<u64>,
    pub(super) stream_id: u64,
    pub(super) sequence: u64,
    pub(super) epoch: u64,
    pub(super) geometry_revision: u64,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct Metrics {
    pub(super) measurement_ns: u64,
    pub(super) ocr_admissions: u64,
    pub(super) ocr_committed: u64,
    pub(super) caller_mapped_bytes: u64,
    pub(super) max_mapping_bytes: u64,
    pub(super) max_retained_layout_bytes: u64,
    pub(super) process: Option<SampledMetrics>,
    pub(super) gpu_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct Report {
    pub(super) schema: u32,
    pub(super) case: &'static str,
    pub(super) semantic_status: &'static str,
    pub(super) cleanup_status: &'static str,
    pub(super) reason: Option<&'static str>,
    pub(super) permission: &'static str,
    pub(super) startup_ns: Option<u64>,
    pub(super) pacing: Vec<Pacing>,
    pub(super) checks: BTreeMap<&'static str, &'static str>,
    pub(super) metrics: Metrics,
    pub(super) samples: Vec<Sample>,
}

impl Report {
    pub(super) fn new() -> Self {
        let keys = [
            "source_default",
            "required_applied",
            "preferred_override",
            "owned_source",
            "completion_cooldown_live_idle",
            "burst_latest",
            "static_observations",
            "resize_retained_mapping",
            "deadline",
            "cancellation",
            "session_close",
            "target_close",
            "source_identity",
            "pacing_report",
            "cooldown_spacing",
            "sample_capacity",
        ];
        Self {
            schema: 2,
            case: "unselected",
            semantic_status: "not-run",
            cleanup_status: "pass",
            reason: None,
            permission: "unavailable",
            startup_ns: None,
            pacing: Vec::with_capacity(4),
            checks: keys.into_iter().map(|key| (key, "not-run")).collect(),
            metrics: Metrics {
                gpu_reason: Some("per-process-gpu-unavailable".to_owned()),
                ..Metrics::default()
            },
            samples: Vec::with_capacity(SAMPLE_LIMIT),
        }
    }

    pub(super) fn check(&mut self, name: &'static str, condition: bool) -> Check<()> {
        self.checks
            .insert(name, if condition { "pass" } else { "fail" });
        require(condition, name)
    }

    pub(super) fn failed(&mut self, failure: Failure) {
        self.semantic_status = if matches!(failure, Failure::NotRun(_)) {
            "not-run"
        } else {
            "fail"
        };
        self.reason = Some(failure.reason());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Vec<OsString> {
        [
            "--case",
            "baseline",
            "--fixture",
            "fixture",
            "--control-root",
            "root",
            "--nonce",
            "0123456789abcdef",
            "--image",
            "image",
            "--model-root",
            "model",
            "--runtime",
            "runtime",
        ]
        .into_iter()
        .map(OsString::from)
        .collect()
    }

    #[test]
    fn execution_requires_every_explicit_argument_without_duplicates() {
        assert!(Arguments::parse(Vec::<OsString>::new()).is_err());
        for index in (0..14).step_by(2) {
            let mut missing = args();
            missing.drain(index..index + 2);
            assert!(Arguments::parse(missing).is_err());
        }
        let mut duplicate = args();
        duplicate.extend([OsString::from("--case"), OsString::from("semantic")]);
        assert!(Arguments::parse(duplicate).is_err());
        assert_eq!(
            Arguments::parse(args()).expect("explicit arguments").case,
            Case::Baseline
        );
    }

    #[test]
    fn nonce_and_case_cannot_select_implicit_or_ambiguous_authority() {
        for nonce in [
            "0123456789ABCDEF",
            "0123456789abcde",
            "0123456789abcdef0",
            "0123456789abcdeg",
        ] {
            let mut invalid = args();
            invalid[7] = nonce.into();
            assert!(Arguments::parse(invalid).is_err());
        }
        let mut invalid = args();
        invalid[1] = "native".into();
        assert!(Arguments::parse(invalid).is_err());
    }

    #[test]
    fn comparison_modes_separate_native_configuration_from_completion_cooldown() {
        use mado_pilot::ResolvedCapturePacing;
        for (case, native, cooldown) in [
            (Case::CaptureOff, false, Duration::ZERO),
            (Case::Baseline, false, Duration::ZERO),
            (Case::CooldownOnly, false, COOLDOWN),
            (Case::NativeOnly, true, Duration::ZERO),
            (Case::Combined, true, COOLDOWN),
        ] {
            let pacing = case
                .pacing()
                .expect("pacing")
                .resolve(ResolvedCapturePacing::source_default());
            assert_eq!(pacing.is_required(), native);
            assert_eq!(pacing.interval(), native.then_some(NATIVE_INTERVAL));
            assert_eq!(case.cooldown(), cooldown);
        }
    }
}
