//! All JSON policy is caller-owned and bounded before native construction.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mado_pilot::{
    CapturePacingRequest, CoordinateSpace, FocusPolicy, InputDelivery, InputOperationKind, Key,
    Modifier, Rect,
};
use serde::Deserialize;

use crate::{Failure, Result};

pub(crate) const MAX_CONFIG_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub target: Target,
    pub ocr: Ocr,
    pub before: Predicate,
    pub action: Action,
    pub input: Option<Input>,
    pub after: Option<Predicate>,
    pub budgets: Budgets,
    pub analysis_interval_ms: u64,
    pub capture_pacing: Option<Pacing>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub window_title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ocr {
    pub model_root: PathBuf,
    pub runtime_path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Predicate {
    pub roi: Roi,
    pub literal: String,
    pub minimum_confidence: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Roi {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Roi {
    pub fn rect(&self) -> Result<Rect> {
        Rect::new(
            CoordinateSpace::CapturePixels,
            f64::from(self.x),
            f64::from(self.y),
            f64::from(self.x) + f64::from(self.width),
            f64::from(self.y) + f64::from(self.height),
        )
        .map_err(|error| Failure::library("roi", error.into()))
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Observe,
    Click,
    Text {
        text: String,
    },
    Chord {
        key: String,
        modifiers: Vec<KeyModifier>,
    },
}

impl Action {
    pub fn operation(&self) -> Option<InputOperationKind> {
        match self {
            Self::Observe => None,
            Self::Click => Some(InputOperationKind::Pointer),
            Self::Text { .. } => Some(InputOperationKind::Text),
            Self::Chord { .. } => Some(InputOperationKind::Keyboard),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeyModifier {
    Shift,
    Control,
    Alt,
    Meta,
}

impl KeyModifier {
    pub fn key(self) -> Key {
        Key::Modifier(match self {
            Self::Shift => Modifier::Shift,
            Self::Control => Modifier::Control,
            Self::Alt => Modifier::Alt,
            Self::Meta => Modifier::Meta,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub route: Route,
    pub focus: Focus,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Route {
    System,
    WindowMessage,
    ProcessDirected,
}

impl Route {
    pub fn delivery(self) -> InputDelivery {
        match self {
            Self::System => InputDelivery::System,
            Self::WindowMessage => InputDelivery::WindowMessage,
            Self::ProcessDirected => InputDelivery::ProcessDirected,
        }
    }

    fn supported_platform(self) -> bool {
        match self {
            Self::System => cfg!(any(windows, target_os = "macos")),
            Self::WindowMessage => cfg!(windows),
            Self::ProcessDirected => cfg!(target_os = "macos"),
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Focus {
    Preserve,
    RequireFocused,
}

impl Focus {
    pub fn policy(self) -> FocusPolicy {
        match self {
            Self::Preserve => FocusPolicy::Preserve,
            Self::RequireFocused => FocusPolicy::RequireFocused,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Budgets {
    pub workflow_ms: u64,
    pub wait_ms: u64,
    pub postcondition_ms: u64,
    pub close_ms: u64,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Pacing {
    SourceDefault,
    Required { minimum_interval_ms: u64 },
    Preferred { minimum_interval_ms: u64 },
}

impl Config {
    pub fn pacing(&self) -> Result<CapturePacingRequest> {
        match &self.capture_pacing {
            None | Some(Pacing::SourceDefault) => Ok(CapturePacingRequest::source_default()),
            Some(Pacing::Required {
                minimum_interval_ms,
            }) => CapturePacingRequest::required(Duration::from_millis(*minimum_interval_ms))
                .map_err(|error| Failure::library("capture_pacing", error)),
            Some(Pacing::Preferred {
                minimum_interval_ms,
            }) => CapturePacingRequest::preferred(Duration::from_millis(*minimum_interval_ms))
                .map_err(|error| Failure::library("capture_pacing", error)),
        }
    }

    fn validate(&self) -> Result<()> {
        bounded_text(&self.target.window_title, 512, "target_title_bounds")?;
        for path in [&self.ocr.model_root, &self.ocr.runtime_path] {
            if !path.is_absolute() || path.as_os_str().len() > 4096 {
                return Err(Failure::Policy("absolute_controlled_ocr_paths_required"));
            }
        }
        self.before.validate()?;
        let b = &self.budgets;
        if !(1..=120_000).contains(&b.workflow_ms)
            || !(1..=b.workflow_ms).contains(&b.wait_ms)
            || !(1..=b.workflow_ms).contains(&b.postcondition_ms)
            || !(1..=10_000).contains(&b.close_ms)
            || !(1..=10_000).contains(&self.analysis_interval_ms)
        {
            return Err(Failure::Policy("timing_bounds"));
        }
        if let Some(
            Pacing::Required {
                minimum_interval_ms,
            }
            | Pacing::Preferred {
                minimum_interval_ms,
            },
        ) = &self.capture_pacing
            && !(1..=10_000).contains(minimum_interval_ms)
        {
            return Err(Failure::Policy("capture_pacing_bounds"));
        }
        match &self.action {
            Action::Observe => {
                if self.input.is_some() || self.after.is_some() {
                    return Err(Failure::Policy("observe_has_no_input_or_postcondition"));
                }
            }
            action => {
                let input = self
                    .input
                    .as_ref()
                    .ok_or(Failure::Policy("input_policy_required"))?;
                if !input.route.supported_platform() {
                    return Err(Failure::Policy("route_unsupported_on_this_platform"));
                }
                self.after
                    .as_ref()
                    .ok_or(Failure::Policy("postcondition_required"))?
                    .validate()?;
                match action {
                    Action::Text { text } => bounded_text(text, 1024, "input_text_bounds")?,
                    Action::Chord { key, modifiers } => {
                        parse_key(key)?;
                        if modifiers.len() > 4
                            || modifiers
                                .iter()
                                .enumerate()
                                .any(|(i, m)| modifiers[..i].contains(m))
                        {
                            return Err(Failure::Policy("chord_modifier_bounds_or_duplicates"));
                        }
                    }
                    Action::Click | Action::Observe => {}
                }
            }
        }
        Ok(())
    }
}

impl Predicate {
    fn validate(&self) -> Result<()> {
        bounded_text(&self.literal, 256, "ocr_literal_bounds")?;
        let r = &self.roi;
        if r.width == 0
            || r.height == 0
            || r.width > 2048
            || r.height > 2048
            || r.x > 16_384
            || r.y > 16_384
            || u64::from(r.width) * u64::from(r.height) > 1_048_576
            || !self.minimum_confidence.is_finite()
            || !(0.0..=1.0).contains(&self.minimum_confidence)
        {
            return Err(Failure::Policy("ocr_predicate_bounds"));
        }
        self.roi.rect()?;
        Ok(())
    }
}

fn bounded_text(text: &str, max: usize, stage: &'static str) -> Result<()> {
    if text.len() > max || text.trim().is_empty() || text.contains('\0') {
        return Err(Failure::Policy(stage));
    }
    Ok(())
}

pub(crate) fn parse_key(value: &str) -> Result<Key> {
    let named = match value {
        "Enter" => Some(Key::Enter),
        "Tab" => Some(Key::Tab),
        "Backspace" => Some(Key::Backspace),
        "Delete" => Some(Key::Delete),
        "Escape" => Some(Key::Escape),
        "Space" => Some(Key::Space),
        "ArrowUp" => Some(Key::ArrowUp),
        "ArrowDown" => Some(Key::ArrowDown),
        "ArrowLeft" => Some(Key::ArrowLeft),
        "ArrowRight" => Some(Key::ArrowRight),
        "Home" => Some(Key::Home),
        "End" => Some(Key::End),
        "PageUp" => Some(Key::PageUp),
        "PageDown" => Some(Key::PageDown),
        _ => None,
    };
    if let Some(key) = named {
        return Ok(key);
    }
    let mut chars = value.chars();
    if let Some(ch) = chars.next()
        && chars.next().is_none()
        && !ch.is_control()
        && !ch.is_whitespace()
    {
        return Ok(Key::Character(ch));
    }
    Err(Failure::Policy(
        "key_must_be_named_or_one_printable_character",
    ))
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Config> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(Failure::Policy("config_exceeds_64_kib"));
    }
    // Serde errors can quote unknown fields or payload values: never print them.
    let config: Config =
        serde_json::from_slice(bytes).map_err(|_| Failure::Policy("config_schema"))?;
    config.validate()?;
    Ok(config)
}

pub(crate) fn load(path: &Path) -> Result<Config> {
    let file = File::open(path).map_err(|_| Failure::Policy("config_open"))?;
    let metadata = file
        .metadata()
        .map_err(|_| Failure::Policy("config_metadata"))?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(Failure::Policy(
            "config_requires_regular_file_at_most_64_kib",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Policy("config_read"))?;
    decode(&bytes)
}
