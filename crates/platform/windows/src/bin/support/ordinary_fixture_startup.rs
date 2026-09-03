//! Closed startup records and activation sequencing shared by the ordinary fixture and test.

use std::fmt;
use std::str::FromStr;

pub(crate) const PREFIX: &str = "fixture-startup-error ";
pub(crate) const MAX_RECORD_BYTES: usize = 384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    DpiAwareness,
    ModuleHandle,
    ClassRegistration,
    WindowCreateTarget,
    WindowCreateGame,
    WindowCreateSibling,
    WindowCreateChild,
    WindowCreateForeground,
    WindowCreateRaw,
    WindowCreateState,
    ForegroundAttach,
    ForegroundRequest,
    ForegroundDetach,
    RawInputRegistration,
    StateTimer,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Self::DpiAwareness => "dpi-awareness",
            Self::ModuleHandle => "module-handle",
            Self::ClassRegistration => "class-registration",
            Self::WindowCreateTarget => "window-create-target",
            Self::WindowCreateGame => "window-create-game",
            Self::WindowCreateSibling => "window-create-sibling",
            Self::WindowCreateChild => "window-create-child",
            Self::WindowCreateForeground => "window-create-foreground",
            Self::WindowCreateRaw => "window-create-raw",
            Self::WindowCreateState => "window-create-state",
            Self::ForegroundAttach => "foreground-attach",
            Self::ForegroundRequest => "foreground-request",
            Self::ForegroundDetach => "foreground-detach",
            Self::RawInputRegistration => "raw-input-registration",
            Self::StateTimer => "state-timer",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "dpi-awareness" => Some(Self::DpiAwareness),
            "module-handle" => Some(Self::ModuleHandle),
            "class-registration" => Some(Self::ClassRegistration),
            "window-create-target" => Some(Self::WindowCreateTarget),
            "window-create-game" => Some(Self::WindowCreateGame),
            "window-create-sibling" => Some(Self::WindowCreateSibling),
            "window-create-child" => Some(Self::WindowCreateChild),
            "window-create-foreground" => Some(Self::WindowCreateForeground),
            "window-create-raw" => Some(Self::WindowCreateRaw),
            "window-create-state" => Some(Self::WindowCreateState),
            "foreground-attach" => Some(Self::ForegroundAttach),
            "foreground-request" => Some(Self::ForegroundRequest),
            "foreground-detach" => Some(Self::ForegroundDetach),
            "raw-input-registration" => Some(Self::RawInputRegistration),
            "state-timer" => Some(Self::StateTimer),
            _ => None,
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DpiAfterFailure {
    NotObserved,
    Unaware,
    System,
    PerMonitor,
    PerMonitorV2,
    Unknown,
}

impl DpiAfterFailure {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotObserved => "not-observed",
            Self::Unaware => "unaware",
            Self::System => "system",
            Self::PerMonitor => "per-monitor",
            Self::PerMonitorV2 => "per-monitor-v2",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "not-observed" => Some(Self::NotObserved),
            "unaware" => Some(Self::Unaware),
            "system" => Some(Self::System),
            "per-monitor" => Some(Self::PerMonitor),
            "per-monitor-v2" => Some(Self::PerMonitorV2),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Foreground {
    Unknown,
    Present,
    Absent,
    SelfThread,
}

impl Foreground {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Present => "present",
            Self::Absent => "absent",
            Self::SelfThread => "self",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "present" => Some(Self::Present),
            "absent" => Some(Self::Absent),
            "self" => Some(Self::SelfThread),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attach {
    NotReached,
    Attempted,
    Skipped,
}

impl Attach {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotReached => "not-reached",
            Self::Attempted => "attempted",
            Self::Skipped => "skipped",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "not-reached" => Some(Self::NotReached),
            "attempted" => Some(Self::Attempted),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectRequest {
    NotReached,
    Accepted,
    Refused,
}

impl DirectRequest {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotReached => "not-reached",
            Self::Accepted => "accepted",
            Self::Refused => "refused",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "not-reached" => Some(Self::NotReached),
            "accepted" => Some(Self::Accepted),
            "refused" => Some(Self::Refused),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationPath {
    Direct,
    Attached,
}

impl ActivationPath {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Attached => "attached",
        }
    }
}

pub(crate) fn ready_record(
    class: &str,
    title: &str,
    capacity: usize,
    activation: Option<ActivationPath>,
) -> String {
    match activation {
        Some(path) => format!(
            "fixture-ready class={class} title={title} capacity={capacity} activation={}",
            path.as_str()
        ),
        None => format!("fixture-ready class={class} title={title} capacity={capacity}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Context {
    dpi_after_failure: DpiAfterFailure,
    foreground: Foreground,
    direct_request: DirectRequest,
    attach: Attach,
}

impl Context {
    pub(crate) const fn new() -> Self {
        Self {
            dpi_after_failure: DpiAfterFailure::NotObserved,
            foreground: Foreground::Unknown,
            direct_request: DirectRequest::NotReached,
            attach: Attach::NotReached,
        }
    }

    pub(crate) const fn with_dpi_after_failure(mut self, dpi: DpiAfterFailure) -> Self {
        self.dpi_after_failure = dpi;
        self
    }

    pub(crate) const fn with_foreground(mut self, foreground: Foreground) -> Self {
        self.foreground = foreground;
        self
    }

    const fn with_direct_request(mut self, direct_request: DirectRequest) -> Self {
        self.direct_request = direct_request;
        self
    }

    const fn with_attach(mut self, attach: Attach) -> Self {
        self.attach = attach;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    WindowsHresult(u32),
    Boolean { ambient_win32: u32 },
}

impl Status {
    fn kind(self) -> &'static str {
        match self {
            Self::WindowsHresult(_) => "windows-hresult",
            Self::Boolean { .. } => "boolean",
        }
    }

    fn code(self) -> Option<u32> {
        match self {
            Self::WindowsHresult(code) => Some(code),
            Self::Boolean { .. } => None,
        }
    }

    fn ambient_win32(self) -> Option<u32> {
        match self {
            Self::WindowsHresult(_) => None,
            Self::Boolean { ambient_win32 } => Some(ambient_win32),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Failure {
    stage: Stage,
    status: Status,
    context: Context,
    cleanup: Option<(Stage, Status)>,
}

impl Failure {
    pub(crate) const fn new(stage: Stage, status: Status, context: Context) -> Self {
        Self {
            stage,
            status,
            context,
            cleanup: None,
        }
    }

    pub(crate) const fn with_cleanup(mut self, stage: Stage, status: Status) -> Self {
        self.cleanup = Some((stage, status));
        self
    }
}

pub(crate) fn activate_with_fallback(
    context: Context,
    may_attach: bool,
    mut request: impl FnMut() -> Result<(), Status>,
    attach_input: impl FnOnce() -> Result<(), Status>,
    detach_input: impl FnOnce() -> Result<(), Status>,
) -> Result<(Context, ActivationPath), Failure> {
    let direct_failure = match request() {
        Ok(()) => {
            return Ok((
                context
                    .with_direct_request(DirectRequest::Accepted)
                    .with_attach(Attach::Skipped),
                ActivationPath::Direct,
            ));
        }
        Err(status) => status,
    };
    let context = context.with_direct_request(DirectRequest::Refused);
    if !may_attach {
        return Err(Failure::new(
            Stage::ForegroundRequest,
            direct_failure,
            context.with_attach(Attach::Skipped),
        ));
    }

    let context = context.with_attach(Attach::Attempted);
    attach_input().map_err(|status| Failure::new(Stage::ForegroundAttach, status, context))?;
    let request_failure = request().err();
    let detach_failure = detach_input().err();
    complete_attached_activation(context, request_failure, detach_failure)
}

fn complete_attached_activation(
    context: Context,
    request_failure: Option<Status>,
    detach_failure: Option<Status>,
) -> Result<(Context, ActivationPath), Failure> {
    match (request_failure, detach_failure) {
        (Some(primary), Some(cleanup)) => {
            Err(Failure::new(Stage::ForegroundRequest, primary, context)
                .with_cleanup(Stage::ForegroundDetach, cleanup))
        }
        (Some(primary), None) => Err(Failure::new(Stage::ForegroundRequest, primary, context)),
        (None, Some(status)) => Err(Failure::new(Stage::ForegroundDetach, status, context)),
        (None, None) => Ok((context, ActivationPath::Attached)),
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (cleanup_stage, cleanup_kind, cleanup_code) = self
            .cleanup
            .map_or(("none", "none", None), |(stage, status)| {
                (stage.as_str(), status.kind(), status.code())
            });
        write!(
            formatter,
            "{PREFIX}version=2 stage={} primary-kind={} primary-code={} foreground={} attach={} direct-request={} dpi-after-failure={} ambient-win32={} cleanup-stage={cleanup_stage} cleanup-kind={cleanup_kind} cleanup-code={}",
            self.stage,
            self.status.kind(),
            Code(self.status.code()),
            self.context.foreground.as_str(),
            self.context.attach.as_str(),
            self.context.direct_request.as_str(),
            self.context.dpi_after_failure.as_str(),
            Code(self.status.ambient_win32()),
            Code(cleanup_code),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseError(&'static str);

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl FromStr for Failure {
    type Err = ParseError;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        if !line.is_ascii() {
            return Err(ParseError("record is not ASCII"));
        }
        if line.len() > MAX_RECORD_BYTES {
            return Err(ParseError("record exceeds byte bound"));
        }
        let mut fields = line.split_ascii_whitespace();
        if fields.next() != Some(PREFIX.trim_end()) {
            return Err(ParseError("record prefix is invalid"));
        }
        if field(&mut fields, "version=")? != "2" {
            return Err(ParseError("record version is unsupported"));
        }
        let stage = Stage::parse(field(&mut fields, "stage=")?)
            .ok_or(ParseError("record stage is unknown"))?;
        let primary_kind = field(&mut fields, "primary-kind=")?;
        let primary_code = field(&mut fields, "primary-code=")?;
        let foreground = Foreground::parse(field(&mut fields, "foreground=")?)
            .ok_or(ParseError("foreground condition is invalid"))?;
        let attach = Attach::parse(field(&mut fields, "attach=")?)
            .ok_or(ParseError("attach condition is invalid"))?;
        let direct_request = DirectRequest::parse(field(&mut fields, "direct-request=")?)
            .ok_or(ParseError("direct-request condition is invalid"))?;
        let dpi_after_failure = DpiAfterFailure::parse(field(&mut fields, "dpi-after-failure=")?)
            .ok_or(ParseError("DPI condition is invalid"))?;
        let ambient_win32 = field(&mut fields, "ambient-win32=")?;
        let status = parse_status(primary_kind, primary_code, ambient_win32)?;
        let cleanup_stage = field(&mut fields, "cleanup-stage=")?;
        let cleanup_status = parse_cleanup_status(
            field(&mut fields, "cleanup-kind=")?,
            field(&mut fields, "cleanup-code=")?,
        )?;
        if fields.next().is_some() {
            return Err(ParseError("record has extra fields"));
        }

        match (stage, status) {
            (Stage::ForegroundRequest, Status::Boolean { .. }) => {}
            (stage, Status::WindowsHresult(_)) if stage != Stage::ForegroundRequest => {}
            _ => return Err(ParseError("primary status provenance is invalid")),
        }
        if (stage == Stage::DpiAwareness) == (dpi_after_failure == DpiAfterFailure::NotObserved) {
            return Err(ParseError("DPI observation stage is invalid"));
        }
        let cleanup = match (cleanup_stage, cleanup_status) {
            ("none", None) => None,
            ("foreground-detach", Some(status)) => Some((Stage::ForegroundDetach, status)),
            _ => return Err(ParseError("cleanup status is invalid")),
        };
        let activation_context_valid = match (direct_request, attach) {
            (DirectRequest::NotReached, Attach::NotReached) => foreground == Foreground::Unknown,
            (DirectRequest::Accepted, Attach::Skipped) => foreground != Foreground::Unknown,
            (DirectRequest::Refused, Attach::Skipped) => {
                matches!(foreground, Foreground::Absent | Foreground::SelfThread)
            }
            (DirectRequest::Refused, Attach::Attempted) => foreground == Foreground::Present,
            _ => false,
        };
        if !activation_context_valid {
            return Err(ParseError("activation context is invalid"));
        }
        let activation_stage_valid = match stage {
            Stage::ForegroundAttach | Stage::ForegroundDetach => {
                direct_request == DirectRequest::Refused && attach == Attach::Attempted
            }
            Stage::ForegroundRequest => {
                direct_request == DirectRequest::Refused
                    && matches!(attach, Attach::Skipped | Attach::Attempted)
            }
            _ => true,
        };
        if !activation_stage_valid
            || (cleanup.is_some()
                && (direct_request != DirectRequest::Refused
                    || attach != Attach::Attempted
                    || foreground != Foreground::Present))
        {
            return Err(ParseError("activation stage is invalid"));
        }
        Ok(Self {
            stage,
            status,
            context: Context {
                dpi_after_failure,
                foreground,
                direct_request,
                attach,
            },
            cleanup,
        })
    }
}

struct Code(Option<u32>);

impl fmt::Display for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(code) => write!(formatter, "0x{code:08X}"),
            None => formatter.write_str("none"),
        }
    }
}

fn field<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    prefix: &str,
) -> Result<&'a str, ParseError> {
    fields
        .next()
        .and_then(|field| field.strip_prefix(prefix))
        .ok_or(ParseError("record fields are missing or reordered"))
}

fn parse_status(kind: &str, code: &str, ambient: &str) -> Result<Status, ParseError> {
    match (kind, parse_code(code)?, parse_code(ambient)?) {
        ("windows-hresult", Some(code), None) => Ok(Status::WindowsHresult(code)),
        ("boolean", None, Some(ambient_win32)) => Ok(Status::Boolean { ambient_win32 }),
        _ => Err(ParseError("status fields disagree")),
    }
}

fn parse_cleanup_status(kind: &str, code: &str) -> Result<Option<Status>, ParseError> {
    match (kind, parse_code(code)?) {
        ("windows-hresult", Some(code)) => Ok(Some(Status::WindowsHresult(code))),
        ("none", None) => Ok(None),
        _ => Err(ParseError("cleanup status fields disagree")),
    }
}

fn parse_code(value: &str) -> Result<Option<u32>, ParseError> {
    if value == "none" {
        return Ok(None);
    }
    let digits = value
        .strip_prefix("0x")
        .ok_or(ParseError("status code prefix is invalid"))?;
    if digits.len() != 8
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    {
        return Err(ParseError("status code is invalid"));
    }
    u32::from_str_radix(digits, 16)
        .map(Some)
        .map_err(|_| ParseError("status code is invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAGES: [Stage; 15] = [
        Stage::DpiAwareness,
        Stage::ModuleHandle,
        Stage::ClassRegistration,
        Stage::WindowCreateTarget,
        Stage::WindowCreateGame,
        Stage::WindowCreateSibling,
        Stage::WindowCreateChild,
        Stage::WindowCreateForeground,
        Stage::WindowCreateRaw,
        Stage::WindowCreateState,
        Stage::ForegroundAttach,
        Stage::ForegroundRequest,
        Stage::ForegroundDetach,
        Stage::RawInputRegistration,
        Stage::StateTimer,
    ];

    fn fallback_context() -> Context {
        Context::new()
            .with_foreground(Foreground::Present)
            .with_direct_request(DirectRequest::Refused)
            .with_attach(Attach::Attempted)
    }

    fn context(stage: Stage) -> Context {
        match stage {
            Stage::DpiAwareness => Context::new().with_dpi_after_failure(DpiAfterFailure::Unaware),
            Stage::ForegroundAttach | Stage::ForegroundRequest | Stage::ForegroundDetach => {
                fallback_context()
            }
            Stage::RawInputRegistration | Stage::StateTimer => Context::new()
                .with_foreground(Foreground::Present)
                .with_direct_request(DirectRequest::Accepted)
                .with_attach(Attach::Skipped),
            _ => Context::new(),
        }
    }

    #[test]
    fn ready_records_preserve_nonactivated_output_and_attribute_activation() {
        let expected = "fixture-ready class=ordinary title=fixture capacity=512";
        assert_eq!(ready_record("ordinary", "fixture", 512, None), expected);
        assert_eq!(
            ready_record("ordinary", "fixture", 512, Some(ActivationPath::Direct)),
            format!("{expected} activation=direct")
        );
        assert_eq!(
            ready_record("ordinary", "fixture", 512, Some(ActivationPath::Attached)),
            format!("{expected} activation=attached")
        );
    }

    #[test]
    fn every_stage_round_trips_with_its_status_provenance() {
        for stage in STAGES {
            let status = if stage == Stage::ForegroundRequest {
                Status::Boolean { ambient_win32: 5 }
            } else {
                Status::WindowsHresult(0x8007_0005)
            };
            let failure = Failure::new(stage, status, context(stage));
            let line = failure.to_string();
            assert!(line.len() <= MAX_RECORD_BYTES);
            assert_eq!(line.parse::<Failure>(), Ok(failure));
        }
    }

    #[test]
    fn boolean_primary_preserves_detach_cleanup() {
        let failure = Failure::new(
            Stage::ForegroundRequest,
            Status::Boolean { ambient_win32: 5 },
            context(Stage::ForegroundRequest),
        )
        .with_cleanup(Stage::ForegroundDetach, Status::WindowsHresult(0x8007_0006));
        assert_eq!(failure.to_string().parse::<Failure>(), Ok(failure));
    }

    #[test]
    fn refused_direct_request_context_round_trips() {
        assert_eq!(ActivationPath::Direct.as_str(), "direct");
        assert_eq!(ActivationPath::Attached.as_str(), "attached");

        let failure = Failure::new(
            Stage::ForegroundAttach,
            Status::WindowsHresult(0x8007_0005),
            Context::new()
                .with_foreground(Foreground::Present)
                .with_direct_request(DirectRequest::Refused)
                .with_attach(Attach::Attempted),
        );
        assert_eq!(failure.to_string().parse::<Failure>(), Ok(failure));
    }

    #[test]
    fn accepted_direct_request_bypasses_failing_attachment() {
        let calls = std::cell::RefCell::new(Vec::new());
        let result = activate_with_fallback(
            Context::new().with_foreground(Foreground::Present),
            true,
            || {
                calls.borrow_mut().push("request");
                Ok(())
            },
            || {
                calls.borrow_mut().push("attach");
                Err(Status::WindowsHresult(0x8007_0005))
            },
            || {
                calls.borrow_mut().push("detach");
                Ok(())
            },
        );

        assert_eq!(calls.into_inner(), ["request"]);
        assert_eq!(
            result,
            Ok((
                Context::new()
                    .with_foreground(Foreground::Present)
                    .with_direct_request(DirectRequest::Accepted)
                    .with_attach(Attach::Skipped),
                ActivationPath::Direct,
            ))
        );
    }

    #[test]
    fn refused_direct_request_uses_one_balanced_attachment() {
        let calls = std::cell::RefCell::new(Vec::new());
        let mut requests = [Err(Status::Boolean { ambient_win32: 5 }), Ok(())].into_iter();
        let result = activate_with_fallback(
            Context::new().with_foreground(Foreground::Present),
            true,
            || {
                calls.borrow_mut().push("request");
                requests.next().expect("two foreground requests")
            },
            || {
                calls.borrow_mut().push("attach");
                Ok(())
            },
            || {
                calls.borrow_mut().push("detach");
                Ok(())
            },
        );

        assert_eq!(
            calls.into_inner(),
            ["request", "attach", "request", "detach"]
        );
        assert_eq!(result, Ok((fallback_context(), ActivationPath::Attached)));
    }

    #[test]
    fn attachment_failure_preserves_refused_direct_request() {
        let calls = std::cell::RefCell::new(Vec::new());
        let result = activate_with_fallback(
            Context::new().with_foreground(Foreground::Present),
            true,
            || {
                calls.borrow_mut().push("request");
                Err(Status::Boolean { ambient_win32: 5 })
            },
            || {
                calls.borrow_mut().push("attach");
                Err(Status::WindowsHresult(0x8007_0005))
            },
            || {
                calls.borrow_mut().push("detach");
                Ok(())
            },
        );

        assert_eq!(calls.into_inner(), ["request", "attach"]);
        assert_eq!(
            result,
            Err(Failure::new(
                Stage::ForegroundAttach,
                Status::WindowsHresult(0x8007_0005),
                fallback_context(),
            ))
        );
    }

    #[test]
    fn attached_request_failure_preserves_detach_cleanup() {
        let calls = std::cell::RefCell::new(Vec::new());
        let mut requests = [
            Err(Status::Boolean { ambient_win32: 5 }),
            Err(Status::Boolean { ambient_win32: 6 }),
        ]
        .into_iter();
        let result = activate_with_fallback(
            Context::new().with_foreground(Foreground::Present),
            true,
            || {
                calls.borrow_mut().push("request");
                requests.next().expect("two foreground requests")
            },
            || {
                calls.borrow_mut().push("attach");
                Ok(())
            },
            || {
                calls.borrow_mut().push("detach");
                Err(Status::WindowsHresult(0x8007_0006))
            },
        );

        assert_eq!(
            calls.into_inner(),
            ["request", "attach", "request", "detach"]
        );
        assert_eq!(
            result,
            Err(Failure::new(
                Stage::ForegroundRequest,
                Status::Boolean { ambient_win32: 6 },
                fallback_context(),
            )
            .with_cleanup(Stage::ForegroundDetach, Status::WindowsHresult(0x8007_0006),))
        );
    }

    #[test]
    fn attached_request_success_still_reports_detach_failure() {
        let mut requests = [Err(Status::Boolean { ambient_win32: 5 }), Ok(())].into_iter();
        let result = activate_with_fallback(
            Context::new().with_foreground(Foreground::Present),
            true,
            || requests.next().expect("two foreground requests"),
            || Ok(()),
            || Err(Status::WindowsHresult(0x8007_0006)),
        );

        assert_eq!(
            result,
            Err(Failure::new(
                Stage::ForegroundDetach,
                Status::WindowsHresult(0x8007_0006),
                fallback_context(),
            ))
        );
    }

    #[test]
    fn refused_direct_request_without_attach_stays_boolean() {
        let result = activate_with_fallback(
            Context::new().with_foreground(Foreground::Absent),
            false,
            || Err(Status::Boolean { ambient_win32: 5 }),
            || panic!("attachment is unavailable"),
            || panic!("detachment requires a successful attachment"),
        );

        assert_eq!(
            result,
            Err(Failure::new(
                Stage::ForegroundRequest,
                Status::Boolean { ambient_win32: 5 },
                Context::new()
                    .with_foreground(Foreground::Absent)
                    .with_direct_request(DirectRequest::Refused)
                    .with_attach(Attach::Skipped),
            ))
        );
    }

    #[test]
    fn parser_rejects_activation_context_stage_and_cleanup_drift() {
        let accepted_with_attachment = Failure::new(
            Stage::ForegroundAttach,
            Status::WindowsHresult(0x8007_0005),
            Context::new()
                .with_foreground(Foreground::Present)
                .with_direct_request(DirectRequest::Accepted)
                .with_attach(Attach::Attempted),
        );
        assert_eq!(
            accepted_with_attachment.to_string().parse::<Failure>(),
            Err(ParseError("activation context is invalid"))
        );

        let skipped_foreign_attachment = Failure::new(
            Stage::ForegroundRequest,
            Status::Boolean { ambient_win32: 0 },
            Context::new()
                .with_foreground(Foreground::Present)
                .with_direct_request(DirectRequest::Refused)
                .with_attach(Attach::Skipped),
        );
        assert_eq!(
            skipped_foreign_attachment.to_string().parse::<Failure>(),
            Err(ParseError("activation context is invalid"))
        );

        let attachment_stage_without_attachment = Failure::new(
            Stage::ForegroundAttach,
            Status::WindowsHresult(0x8007_0005),
            Context::new()
                .with_foreground(Foreground::Absent)
                .with_direct_request(DirectRequest::Refused)
                .with_attach(Attach::Skipped),
        );
        assert_eq!(
            attachment_stage_without_attachment
                .to_string()
                .parse::<Failure>(),
            Err(ParseError("activation stage is invalid"))
        );

        let cleanup_without_attachment = Failure::new(
            Stage::RawInputRegistration,
            Status::WindowsHresult(0x8007_0005),
            context(Stage::RawInputRegistration),
        )
        .with_cleanup(Stage::ForegroundDetach, Status::WindowsHresult(0x8007_0006));
        assert_eq!(
            cleanup_without_attachment.to_string().parse::<Failure>(),
            Err(ParseError("activation stage is invalid"))
        );
    }

    #[test]
    fn parser_rejects_unbounded_free_form_and_provenance_drift() {
        let invalid = Failure::new(
            Stage::ForegroundRequest,
            Status::WindowsHresult(0x8007_0005),
            context(Stage::ForegroundRequest),
        );
        assert_eq!(
            invalid.to_string().parse::<Failure>(),
            Err(ParseError("primary status provenance is invalid"))
        );
        assert_eq!(
            ("x".repeat(MAX_RECORD_BYTES + 1)).parse::<Failure>(),
            Err(ParseError("record exceeds byte bound"))
        );
        assert_eq!(
            format!(
                "{} title=秘密",
                Failure::new(
                    Stage::ModuleHandle,
                    Status::WindowsHresult(0x8007_0005),
                    context(Stage::ModuleHandle),
                )
            )
            .parse::<Failure>(),
            Err(ParseError("record is not ASCII"))
        );
    }
}
