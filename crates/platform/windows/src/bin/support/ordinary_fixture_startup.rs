use std::fmt;
use std::str::FromStr;

pub(crate) const PREFIX: &str = "fixture-startup-error ";
pub(crate) const MAX_RECORD_BYTES: usize = 384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stage(&'static str);

impl Stage {
    pub(crate) const DPI_AWARENESS: Self = Self("dpi-awareness");
    pub(crate) const TITLE_TOKEN: Self = Self("title-token");
    pub(crate) const MODULE_HANDLE: Self = Self("module-handle");
    pub(crate) const CLASS_REGISTRATION: Self = Self("class-registration");
    pub(crate) const WINDOW_CREATE_TARGET: Self = Self("window-create-target");
    pub(crate) const WINDOW_CREATE_GAME: Self = Self("window-create-game");
    pub(crate) const WINDOW_CREATE_SIBLING: Self = Self("window-create-sibling");
    pub(crate) const WINDOW_CREATE_CHILD: Self = Self("window-create-child");
    pub(crate) const WINDOW_CREATE_FOREGROUND: Self = Self("window-create-foreground");
    pub(crate) const WINDOW_CREATE_RAW: Self = Self("window-create-raw");
    pub(crate) const WINDOW_CREATE_STATE: Self = Self("window-create-state");
    pub(crate) const FOREGROUND_ATTACH: Self = Self("foreground-attach");
    pub(crate) const FOREGROUND_REQUEST: Self = Self("foreground-request");
    pub(crate) const FOREGROUND_DETACH: Self = Self("foreground-detach");
    pub(crate) const RAW_INPUT_REGISTRATION: Self = Self("raw-input-registration");
    pub(crate) const STATE_TIMER: Self = Self("state-timer");

    const ALL: [Self; 16] = [
        Self::DPI_AWARENESS,
        Self::TITLE_TOKEN,
        Self::MODULE_HANDLE,
        Self::CLASS_REGISTRATION,
        Self::WINDOW_CREATE_TARGET,
        Self::WINDOW_CREATE_GAME,
        Self::WINDOW_CREATE_SIBLING,
        Self::WINDOW_CREATE_CHILD,
        Self::WINDOW_CREATE_FOREGROUND,
        Self::WINDOW_CREATE_RAW,
        Self::WINDOW_CREATE_STATE,
        Self::FOREGROUND_ATTACH,
        Self::FOREGROUND_REQUEST,
        Self::FOREGROUND_DETACH,
        Self::RAW_INPUT_REGISTRATION,
        Self::STATE_TIMER,
    ];

    fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.0 == value)
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DpiBefore {
    Unaware,
    System,
    PerMonitor,
    PerMonitorV2,
    Unknown,
}

impl DpiBefore {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unaware => "unaware",
            Self::System => "system",
            Self::PerMonitor => "per-monitor",
            Self::PerMonitorV2 => "per-monitor-v2",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
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
pub(crate) struct Context {
    dpi_before: DpiBefore,
    foreground: Foreground,
    attach: Attach,
    ambient_win32: Option<u32>,
}

impl Context {
    pub(crate) const fn new(dpi_before: DpiBefore) -> Self {
        Self {
            dpi_before,
            foreground: Foreground::Unknown,
            attach: Attach::NotReached,
            ambient_win32: None,
        }
    }

    pub(crate) const fn with_activation(mut self, foreground: Foreground, attach: Attach) -> Self {
        self.foreground = foreground;
        self.attach = attach;
        self
    }

    pub(crate) const fn with_ambient_win32(mut self, code: u32) -> Self {
        self.ambient_win32 = Some(code);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    WindowsHresult(u32),
    Hresult(u32),
    Boolean,
}

impl Status {
    fn kind(self) -> &'static str {
        match self {
            Self::WindowsHresult(_) => "windows-hresult",
            Self::Hresult(_) => "hresult",
            Self::Boolean => "boolean",
        }
    }

    fn code(self) -> Option<u32> {
        match self {
            Self::WindowsHresult(code) | Self::Hresult(code) => Some(code),
            Self::Boolean => None,
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

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (cleanup_stage, cleanup_kind, cleanup_code) = self
            .cleanup
            .map_or(("none", "none", None), |(stage, status)| {
                (stage.0, status.kind(), status.code())
            });
        write!(
            formatter,
            "{PREFIX}version=1 stage={} primary-kind={} primary-code={} foreground={} attach={} dpi-before={} ambient-win32={} cleanup-stage={cleanup_stage} cleanup-kind={cleanup_kind} cleanup-code={}",
            self.stage,
            self.status.kind(),
            Code(self.status.code()),
            self.context.foreground.as_str(),
            self.context.attach.as_str(),
            self.context.dpi_before.as_str(),
            Code(self.context.ambient_win32),
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
        if field(&mut fields, "version=")? != "1" {
            return Err(ParseError("record version is unsupported"));
        }
        let stage = Stage::parse(field(&mut fields, "stage=")?)
            .ok_or(ParseError("record stage is unknown"))?;
        let status = parse_status(
            field(&mut fields, "primary-kind=")?,
            field(&mut fields, "primary-code=")?,
        )?
        .ok_or(ParseError("primary status is absent"))?;
        let foreground = Foreground::parse(field(&mut fields, "foreground=")?)
            .ok_or(ParseError("foreground condition is invalid"))?;
        let attach = Attach::parse(field(&mut fields, "attach=")?)
            .ok_or(ParseError("attach condition is invalid"))?;
        let dpi_before = DpiBefore::parse(field(&mut fields, "dpi-before=")?)
            .ok_or(ParseError("DPI condition is invalid"))?;
        let ambient_win32 = parse_code(field(&mut fields, "ambient-win32=")?)?;
        let cleanup_stage = field(&mut fields, "cleanup-stage=")?;
        let cleanup_status = parse_status(
            field(&mut fields, "cleanup-kind=")?,
            field(&mut fields, "cleanup-code=")?,
        )?;
        if fields.next().is_some() {
            return Err(ParseError("record has extra fields"));
        }

        match (stage, status, ambient_win32) {
            (Stage::TITLE_TOKEN, Status::Hresult(_), None)
            | (Stage::FOREGROUND_REQUEST, Status::Boolean, Some(_)) => {}
            (stage, Status::WindowsHresult(_), None)
                if stage != Stage::TITLE_TOKEN && stage != Stage::FOREGROUND_REQUEST => {}
            _ => return Err(ParseError("primary status provenance is invalid")),
        }
        let cleanup = match (cleanup_stage, cleanup_status) {
            ("none", None) => None,
            ("foreground-detach", Some(status @ Status::WindowsHresult(_))) => {
                Some((Stage::FOREGROUND_DETACH, status))
            }
            _ => return Err(ParseError("cleanup status is invalid")),
        };
        Ok(Self {
            stage,
            status,
            context: Context {
                dpi_before,
                foreground,
                attach,
                ambient_win32,
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

fn parse_status(kind: &str, code: &str) -> Result<Option<Status>, ParseError> {
    let code = parse_code(code)?;
    match (kind, code) {
        ("windows-hresult", Some(code)) => Ok(Some(Status::WindowsHresult(code))),
        ("hresult", Some(code)) => Ok(Some(Status::Hresult(code))),
        ("boolean", None) => Ok(Some(Status::Boolean)),
        ("none", None) => Ok(None),
        _ => Err(ParseError("status kind and code disagree")),
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

    fn context() -> Context {
        Context::new(DpiBefore::PerMonitorV2)
            .with_activation(Foreground::Present, Attach::Attempted)
    }

    #[test]
    fn every_stage_round_trips_with_its_status_provenance() {
        for stage in Stage::ALL {
            let failure = match stage {
                Stage::TITLE_TOKEN => Failure::new(stage, Status::Hresult(0x8000_4005), context()),
                Stage::FOREGROUND_REQUEST => {
                    Failure::new(stage, Status::Boolean, context().with_ambient_win32(5))
                }
                _ => Failure::new(stage, Status::WindowsHresult(0x8007_0005), context()),
            };
            let line = failure.to_string();
            assert!(line.len() <= MAX_RECORD_BYTES);
            assert_eq!(line.parse::<Failure>(), Ok(failure));
        }
    }

    #[test]
    fn boolean_primary_preserves_detach_cleanup() {
        let failure = Failure::new(
            Stage::FOREGROUND_REQUEST,
            Status::Boolean,
            context().with_ambient_win32(5),
        )
        .with_cleanup(
            Stage::FOREGROUND_DETACH,
            Status::WindowsHresult(0x8007_0006),
        );
        assert_eq!(failure.to_string().parse::<Failure>(), Ok(failure));
    }

    #[test]
    fn parser_rejects_unbounded_free_form_and_provenance_drift() {
        assert_eq!(
            (Failure::new(
                Stage::FOREGROUND_REQUEST,
                Status::WindowsHresult(0x8007_0005),
                context(),
            ))
            .to_string()
            .parse::<Failure>(),
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
                    Stage::DPI_AWARENESS,
                    Status::WindowsHresult(0x8007_0005),
                    context(),
                )
            )
            .parse::<Failure>(),
            Err(ParseError("record is not ASCII"))
        );
    }
}
