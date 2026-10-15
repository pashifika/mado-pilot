//! Shared build/maintainer catalog validation and deterministic Rust emission.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

macro_rules! record {
    ($name:ident { $($(#[$attribute:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        struct $name { $($(#[$attribute])* $field: $ty),* }
    };
}

record!(Catalog {
    schema_version: u32,
    default_profile: String,
    resources: Vec<Resource>,
    profiles: Vec<Profile>,
});
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resource {
    id: String,
    kind: String,
    pub(crate) relative_path: String,
    pub(crate) byte_len: u64,
    pub(crate) sha256: String,
    pub(crate) source_url: String,
    source_revision: String,
    license: String,
    notice: String,
}
record!(Profile {
    status: String,
    #[serde(deserialize_with = "required_option")]
    legacy_constants: Option<String>,
    execution: Execution,
    presentation: Presentation,
});
record!(Execution {
    model_id: String, model_version: String, profile_id: String,
    language_profile_id: String, preprocessing_id: String, decoder_id: String,
    normalization_id: String, detector: String, recognizer: String,
    #[serde(deserialize_with = "required_option")]
    auxiliary: Option<String>,
    detector_graph: Graph, recognizer_graph: Graph, detection: Detection,
    recognition: Recognition, vocabulary: Vocabulary, limits: Limits,
    applicability: Vec<Applicability>,
});
record!(Graph {
    input: Tensor,
    output: Tensor
});
record!(Tensor { name: String, dtype: String, shape: Vec<i64>, axis_limits: Vec<u32> });
record!(Detection {
    algorithm: String, resize: String, side: u32, multiple: u32, minimum_dimension: u32,
    #[serde(deserialize_with = "required_option")]
    max_width: Option<u32>,
    #[serde(deserialize_with = "required_option")]
    max_height: Option<u32>,
    #[serde(deserialize_with = "required_option")]
    max_tensor_bytes: Option<usize>,
    channel_order: String, interpolation: String, rounding: String, inverse_geometry: String,
    normalization: String, mean: [f32; 3], std: [f32; 3], threshold: f32,
    box_threshold: f64, unclip_ratio: f64, min_side: f32, sort_line_threshold: f32,
    max_candidates: usize, dilation: [u32; 2], score_mode: String,
});
record!(Recognition {
    height: usize,
    nominal_width: usize,
    max_width: usize,
    batch: usize,
    crop: String,
    orientation: String,
    resize: String,
    padding: String,
    normalization: String,
});
record!(Vocabulary {
    source: String,
    key: String,
    entries: u32,
    sha256: String,
    classes: u32,
    decoder: String,
    blank_index: u32,
    append_space: bool,
    remove_adjacent_duplicates: bool,
    confidence: String,
});
record!(Limits {
    input_tensor_bytes: usize,
    output_tensor_bytes: usize,
    contour_bytes: usize,
    scanline_bytes: usize,
    model_bytes: u64,
    auxiliary_bytes: u64,
});
record!(Applicability {
    target: String,
    provider: String,
    runtime_version: String,
    runtime_api: u32
});
record!(Presentation {
    display_name: String,
    #[serde(deserialize_with = "required_option")]
    variant: Option<String>,
    benchmarks: Vec<Benchmark>,
});
record!(Benchmark {
    target: String, provider: String, code: String,
    #[serde(deserialize_with = "required_option")]
    criteria: Option<String>,
    #[serde(deserialize_with = "required_option")]
    report: Option<String>,
});

// serde_json's ordinary Value visitor overwrites duplicate object keys. Reject
// them before typed deserialization, including escaped spellings of the same key.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, UniqueValue(value))) =
                    map.next_entry::<String, UniqueValue>()?
                {
                    if values.insert(key.clone(), value).is_some() {
                        return Err(serde::de::Error::custom(format!("duplicate key: {key}")));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

/// Build-validated catalog; construction always traverses the same strict checks.
#[derive(Debug)]
pub(crate) struct ValidatedCatalog(Catalog, [u8; 32]);

/// Validates one bounded versioned source, rejecting malformed or unimplemented contracts.
pub(crate) fn validate(source: &str) -> Result<ValidatedCatalog, String> {
    require(source.len() <= 1024 * 1024, "catalog exceeds 1 MiB")?;
    let UniqueValue(value) = serde_json::from_str(source).map_err(|error| error.to_string())?;
    let catalog: Catalog = serde_json::from_value(value).map_err(|error| error.to_string())?;
    require(
        catalog.schema_version == 1,
        "unsupported catalog schema version",
    )?;
    require(
        !catalog.resources.is_empty() && catalog.resources.len() <= 128,
        "invalid resource count",
    )?;
    require(
        !catalog.profiles.is_empty() && catalog.profiles.len() <= 64,
        "invalid profile count",
    )?;
    let mut resources = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for resource in &catalog.resources {
        identifier(&resource.id)?;
        safe_path(&resource.relative_path)?;
        require(
            resources.insert(resource.id.as_str(), resource).is_none(),
            "duplicate resource ID",
        )?;
        require(
            paths.insert(resource.relative_path.to_ascii_lowercase()),
            "duplicate resource path",
        )?;
        require(
            matches!(resource.kind.as_str(), "model" | "auxiliary"),
            "unknown resource kind",
        )?;
        let ceiling = if resource.kind == "model" {
            64 * 1024 * 1024
        } else {
            1024 * 1024
        };
        require(
            resource.byte_len > 0 && resource.byte_len <= ceiling,
            "invalid resource length",
        )?;
        digest(&resource.sha256)?;
        require(
            resource.source_revision.len() == 40 && hex(&resource.source_revision),
            "source revision is not immutable",
        )?;
        let source_path = resource
            .source_url
            .strip_prefix("https://huggingface.co/")
            .ok_or("source URL must use the reviewed HTTPS host")?;
        safe_path(source_path)?;
        let mut parts = source_path.split('/');
        let owner = parts.next();
        let repository = parts.next();
        require(
            owner.is_some()
                && repository.is_some()
                && parts.next() == Some("resolve")
                && parts.next() == Some(resource.source_revision.as_str())
                && parts.next().is_some(),
            "source URL is not revision pinned",
        )?;
        require(
            resource.license == "Apache-2.0",
            "unreviewed resource license",
        )?;
        text(&resource.notice, 1024)?;
    }
    let mut profile_ids = BTreeSet::new();
    let mut model_ids = BTreeSet::new();
    let mut prefixes = BTreeSet::new();
    let mut used_resources = BTreeSet::new();
    for profile in &catalog.profiles {
        let e = &profile.execution;
        require(
            profile_ids.insert(e.profile_id.as_str()),
            "duplicate profile ID",
        )?;
        require(model_ids.insert(e.model_id.as_str()), "duplicate model ID")?;
        require(
            matches!(profile.status.as_str(), "accepted" | "candidate"),
            "unknown admission status",
        )?;
        if let Some(prefix) = &profile.legacy_constants {
            require(
                prefixes.insert(prefix.as_str()),
                "duplicate legacy constant namespace",
            )?;
            let released_id = match prefix.as_str() {
                "ACCEPTED_G004" => "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1",
                "ACCEPTED_BOUNDED" => "phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v2",
                _ => return Err("invalid legacy constant namespace".into()),
            };
            require(
                profile.status == "accepted"
                    && e.model_id == released_id
                    && e.profile_id == released_id,
                "invalid legacy constant binding",
            )?;
        }
        validate_execution(e, &resources, &mut used_resources)
            .map_err(|error| format!("{}: {error}", e.profile_id))?;
        validate_presentation(profile)?;
        if profile.status == "candidate" {
            require(
                e.applicability.iter().all(|a| a.provider == "cpu"),
                "candidate provider is not qualified",
            )?;
        }
    }
    require(
        prefixes == BTreeSet::from(["ACCEPTED_G004", "ACCEPTED_BOUNDED"]),
        "both legacy profiles are required",
    )?;
    for profile in &catalog.profiles {
        let e = &profile.execution;
        require(
            e.model_id == e.profile_id || !profile_ids.contains(e.model_id.as_str()),
            "ambiguous closed model/profile ID",
        )?;
    }
    let default = catalog
        .profiles
        .iter()
        .find(|p| p.execution.profile_id == catalog.default_profile)
        .ok_or("default profile reference is missing")?;
    require(
        default.legacy_constants.as_deref() == Some("ACCEPTED_G004"),
        "default must remain native G-004",
    )?;
    require(
        used_resources.len() == resources.len(),
        "unreferenced resource",
    )?;
    Ok(ValidatedCatalog(
        catalog,
        Sha256::digest(source.as_bytes()).into(),
    ))
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

fn text(value: &str, ceiling: usize) -> Result<(), String> {
    require(
        !value.is_empty()
            && value.len() <= ceiling
            && value.trim() == value
            && !value.chars().any(char::is_control),
        "invalid text field",
    )
}

fn identifier(value: &str) -> Result<(), String> {
    text(value, 128)
}

fn hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn digest(value: &str) -> Result<[u8; 32], String> {
    require(
        value.len() == 64 && hex(value),
        "SHA-256 must be 64 lowercase hex characters",
    )?;
    let mut result = [0; 32];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|error| error.to_string())?;
    }
    require(
        result != [0; 32],
        "zero SHA-256 is not an immutable resource identity",
    )?;
    Ok(result)
}

fn safe_path(path: &str) -> Result<(), String> {
    require(
        !path.is_empty() && path.len() <= 512,
        "invalid resource path length",
    )?;
    for part in path.split('/') {
        require(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "unsafe relative resource path",
        )?;
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        require(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "reserved Windows resource path",
        )?;
    }
    Ok(())
}

fn validate_execution<'a>(
    e: &'a Execution,
    resources: &BTreeMap<&str, &'a Resource>,
    used: &mut BTreeSet<&'a str>,
) -> Result<(), String> {
    for id in [
        &e.model_id,
        &e.model_version,
        &e.profile_id,
        &e.language_profile_id,
        &e.preprocessing_id,
        &e.decoder_id,
        &e.normalization_id,
    ] {
        identifier(id)?;
    }
    require(
        e.language_profile_id == "horizontal-ja-basic-latin-ascii-digits-ui-symbols-v1"
            && e.normalization_id == "nfc-trim-stable-detector-order-five-decimal-v1",
        "unsupported language/orientation or text normalization",
    )?;
    let lookup = |id: &str, kind: &str| -> Result<&Resource, String> {
        let resource = *resources
            .get(id)
            .ok_or_else(|| format!("dangling resource reference: {id}"))?;
        require(resource.kind == kind, "resource kind mismatch")?;
        Ok(resource)
    };
    lookup(&e.detector, "model")?;
    lookup(&e.recognizer, "model")?;
    require(
        e.detector != e.recognizer,
        "detector and recognizer must be distinct resources",
    )?;
    used.extend([e.detector.as_str(), e.recognizer.as_str()]);
    if let Some(auxiliary) = &e.auxiliary {
        lookup(auxiliary, "auxiliary")?;
        used.insert(auxiliary);
    }
    let limits = &e.limits;
    require(
        limits.input_tensor_bytes == 256 * 1024 * 1024
            && limits.output_tensor_bytes == 256 * 1024 * 1024
            && limits.contour_bytes == 2 * 1024 * 1024
            && limits.scanline_bytes == 16 * 1024 * 1024
            && limits.model_bytes == 64 * 1024 * 1024
            && limits.auxiliary_bytes == 1024 * 1024,
        "unsupported finite resource ceilings",
    )?;
    let d = &e.detection;
    require(
        d.algorithm == "db-rapidocr-v1"
            && d.channel_order == "bgr"
            && d.interpolation == "opencv-linear-direct"
            && d.rounding == "truncate-then-ties-even"
            && d.inverse_geometry == "source-over-final-xy"
            && d.dilation == [2, 2]
            && d.score_mode == "fast",
        "unsupported detector algorithm",
    )?;
    require(
        d.side > 0 && d.side <= 4096 && d.multiple == 32 && d.side.is_multiple_of(d.multiple),
        "invalid detector sampling dimensions",
    )?;
    require(
        d.threshold.is_finite()
            && d.threshold > 0.0
            && d.threshold < 1.0
            && d.box_threshold.is_finite()
            && d.box_threshold > 0.0
            && d.box_threshold <= 1.0
            && d.unclip_ratio.is_finite()
            && d.unclip_ratio > 0.0
            && d.unclip_ratio <= 10.0
            && d.min_side == 3.0
            && d.sort_line_threshold == 10.0
            && d.max_candidates == 1000,
        "invalid DB parameters",
    )?;
    require(
        d.mean
            .iter()
            .all(|x| x.is_finite() && (0.0..=1.0).contains(x))
            && d.std.iter().all(|x| x.is_finite() && *x > 0.0 && *x <= 1.0),
        "invalid pixel normalization",
    )?;
    match d.normalization.as_str() {
        "legacy-divide" => require(
            d.mean == [0.5; 3] && d.std == [0.5; 3],
            "legacy pixel arithmetic is fixed",
        )?,
        "multiply-mean-std" => (),
        _ => return Err("unsupported pixel operation order".into()),
    }
    match d.resize.as_str() {
        "legacy-short-side" => require(
            d.minimum_dimension == 0
                && d.max_width.is_none()
                && d.max_height.is_none()
                && d.max_tensor_bytes.is_none(),
            "incomplete legacy sampling",
        )?,
        "bounded-rectangular" => {
            require(d.minimum_dimension == 0, "bounded legacy minimum changed")?;
            validate_rectangle(d, limits.input_tensor_bytes)?;
        }
        "max-side" => {
            require(
                d.minimum_dimension == d.multiple
                    && d.max_width == Some(d.side)
                    && d.max_height == Some(d.side),
                "incomplete max-side sampling",
            )?;
            validate_rectangle(d, limits.input_tensor_bytes)?;
            let maximum = u64::from(d.side) * u64::from(d.side) * 12;
            require(
                d.max_tensor_bytes.and_then(|n| u64::try_from(n).ok()) == Some(maximum),
                "max-side tensor bound does not cover its complete shape",
            )?;
        }
        _ => return Err("unsupported detector resize strategy".into()),
    }
    let r = &e.recognition;
    require(
        r.height == 48
            && r.nominal_width == 320
            && r.max_width == 4096
            && r.batch == 6
            && r.crop == "source-perspective-cubic-replicate"
            && r.orientation == "rotate-ccw-if-height-over-width-ge-1.5"
            && r.resize == "opencv-linear-aspect-ceil-width"
            && r.padding == "right-zero"
            && r.normalization == "legacy-divide",
        "unsupported recognition algorithm",
    )?;
    let v = &e.vocabulary;
    digest(&v.sha256)?;
    require(
        v.entries == 18_708
            && v.classes == 18_710
            && v.decoder == "greedy-ctc"
            && v.blank_index == 0
            && v.append_space
            && v.remove_adjacent_duplicates
            && v.confidence == "rounded-token-maxima-mean-five-decimal",
        "invalid CTC vocabulary/decoder contract",
    )?;
    match v.source.as_str() {
        "onnx-metadata" => require(
            v.key == "character" && e.auxiliary.is_none(),
            "metadata vocabulary must not name an auxiliary resource",
        )?,
        "paddle-yaml" => require(
            v.key == "PostProcess.character_dict" && e.auxiliary.is_some(),
            "YAML vocabulary requires its exact auxiliary resource",
        )?,
        _ => return Err("unsupported vocabulary source".into()),
    }
    validate_tensor(&e.detector_graph.input, &[-1, 3, -1, -1])?;
    validate_tensor(&e.detector_graph.output, &[-1, 1, -1, -1])?;
    validate_tensor(&e.recognizer_graph.input, &[-1, 3, 48, -1])?;
    validate_tensor(&e.recognizer_graph.output, &[-1, -1, i64::from(v.classes)])?;
    let input = &e.detector_graph.input.axis_limits;
    let output = &e.detector_graph.output.axis_limits;
    require(
        input[0] == 1 && output[0] == 1 && input[2..] == output[2..],
        "detector graph axes disagree",
    )?;
    match (d.max_width, d.max_height) {
        (Some(width), Some(height)) => require(
            input[2] == height && input[3] == width,
            "sampling and graph ceilings disagree",
        )?,
        (None, None) => require(
            input[2] == 22369621 && input[3] == 22369621,
            "legacy graph must retain checked tensor axis ceiling",
        )?,
        _ => return Err("incomplete graph ceiling".into()),
    }
    let classes = usize::try_from(v.classes).map_err(|_| "class count overflow")?;
    require(
        e.recognizer_graph.input.axis_limits == [6, 3, 48, 4096]
            && e.recognizer_graph.output.axis_limits[0] == 6
            && e.recognizer_graph.output.axis_limits[1]
                == u32::try_from(limits.output_tensor_bytes / 4 / classes)
                    .map_err(|_| "class bound overflow")?,
        "recognizer graph ceilings disagree",
    )?;
    let mut applicability = BTreeSet::new();
    for a in &e.applicability {
        require(
            matches!(
                a.target.as_str(),
                "aarch64-apple-darwin" | "x86_64-pc-windows-msvc"
            ) && (a.provider == "cpu"
                || (a.provider == "cuda" && a.target == "x86_64-pc-windows-msvc"))
                && a.runtime_version == "1.29.0"
                && a.runtime_api == 17,
            "unsupported runtime/provider/target",
        )?;
        require(
            applicability.insert((a.target.as_str(), a.provider.as_str())),
            "duplicate applicability",
        )?;
    }
    require(
        applicability.contains(&("aarch64-apple-darwin", "cpu"))
            && applicability.contains(&("x86_64-pc-windows-msvc", "cpu")),
        "both CPU release targets are required",
    )?;
    Ok(())
}

fn validate_rectangle(d: &Detection, tensor_limit: usize) -> Result<(), String> {
    let (Some(width), Some(height), Some(bytes)) = (d.max_width, d.max_height, d.max_tensor_bytes)
    else {
        return Err("incomplete bounded sampling".into());
    };
    require(
        width > 0
            && height > 0
            && width <= 4096
            && height <= 4096
            && width.is_multiple_of(d.multiple)
            && height.is_multiple_of(d.multiple)
            && bytes >= 32 * 32 * 12
            && bytes <= tensor_limit,
        "invalid bounded sampling ceiling",
    )
}

fn validate_tensor(t: &Tensor, shape: &[i64]) -> Result<(), String> {
    identifier(&t.name)?;
    require(
        t.dtype == "f32" && t.shape == shape && t.axis_limits.len() == shape.len(),
        "unsupported tensor name/type/rank/shape",
    )?;
    for (&axis, &ceiling) in shape.iter().zip(&t.axis_limits) {
        require(
            ceiling > 0 && ceiling <= 67108864 && (axis == -1 || axis == i64::from(ceiling)),
            "invalid tensor axis bound",
        )?;
    }
    Ok(())
}

fn validate_presentation(profile: &Profile) -> Result<(), String> {
    let p = &profile.presentation;
    text(&p.display_name, 256)?;
    if let Some(variant) = &p.variant {
        text(variant, 256)?;
    }
    let mut scopes = BTreeSet::new();
    for b in &p.benchmarks {
        require(
            profile.status == "accepted",
            "candidate cannot publish a comparative support verdict",
        )?;
        require(
            profile
                .execution
                .applicability
                .iter()
                .any(|a| a.target == b.target && a.provider == b.provider),
            "evidence scope is inapplicable",
        )?;
        require(
            scopes.insert((&b.target, &b.provider)),
            "duplicate evidence scope",
        )?;
        match b.code.as_str() {
            "unrated" => require(
                profile.legacy_constants.is_some() && b.criteria.is_none() && b.report.is_none(),
                "only legacy support may remain unrated",
            )?,
            "reference" | "fast" | "balanced" => {
                let (Some(criteria), Some(report)) = (&b.criteria, &b.report) else {
                    return Err(
                        "comparative verdict requires criteria and report references".into(),
                    );
                };
                safe_path(criteria)?;
                safe_path(report)?;
            }
            _ => return Err("unknown benchmark code".into()),
        }
    }
    require(
        profile.status != "accepted" || scopes.len() == profile.execution.applicability.len(),
        "accepted evidence scopes are incomplete",
    )
}

impl ValidatedCatalog {
    /// Canonical execution digests, independent of source layout and presentation.
    pub(crate) fn execution_digests(&self) -> Result<BTreeMap<String, String>, String> {
        self.0
            .profiles
            .iter()
            .map(|profile| {
                Ok((
                    profile.execution.profile_id.clone(),
                    to_hex(&self.execution_digest(profile)?),
                ))
            })
            .collect()
    }

    /// Validated resources in detector, recognizer, auxiliary order.
    pub(crate) fn resources(
        &self,
        profile_id: &str,
    ) -> Result<impl Iterator<Item = &Resource>, String> {
        let execution = &self
            .0
            .profiles
            .iter()
            .find(|profile| profile.execution.profile_id == profile_id)
            .ok_or("undeclared profile ID")?
            .execution;
        Ok([&execution.detector, &execution.recognizer]
            .into_iter()
            .chain(execution.auxiliary.iter())
            .map(|id| {
                self.0
                    .resources
                    .iter()
                    .find(|resource| &resource.id == id)
                    .expect("validated resource reference")
            }))
    }

    fn execution_digest(&self, profile: &Profile) -> Result<[u8; 32], String> {
        let mut execution = profile.execution.clone();
        execution
            .applicability
            .sort_by(|a, b| (&a.target, &a.provider).cmp(&(&b.target, &b.provider)));
        let mut resources = BTreeMap::new();
        for resource in self.resources(&execution.profile_id)? {
            resources.insert(&resource.id, resource);
        }
        let canonical = serde_json::json!({ "schema_version": self.0.schema_version, "execution": execution, "resources": resources });
        let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
        Ok(Sha256::digest(bytes).into())
    }

    /// Emits static tables and released public constants from this validated source.
    pub(crate) fn rust_source(&self) -> Result<String, String> {
        let catalog = &self.0;
        let digests = self.execution_digests()?;
        let mut out = String::new();
        writeln!(out, "#[cfg(feature = \"model-candidates\")]\n#[doc(hidden)]\npub const CATALOG_SOURCE_SHA256: [u8; 32] = {:?};", self.1).unwrap();
        writeln!(out, "/// Unchanged default accepted OCR profile.\npub const DEFAULT_OCR_PROFILE_ID: &str = {:?};", catalog.default_profile).unwrap();
        let limit = catalog.profiles[0].execution.limits.model_bytes;
        writeln!(out, "/// Maximum bytes accepted for one OCR model component.\npub const MAX_MODEL_COMPONENT_BYTES: u64 = {limit};").unwrap();
        for profile in &catalog.profiles {
            if let Some(prefix) = &profile.legacy_constants {
                let e = &profile.execution;
                let fields = [
                    ("MODEL_ID", &e.model_id),
                    ("MODEL_VERSION", &e.model_version),
                    ("PROFILE_ID", &e.profile_id),
                    ("PREPROCESSING_ID", &e.preprocessing_id),
                ];
                for (name, value) in fields {
                    writeln!(out, "/// Catalog-generated released identity.\npub const {prefix}_{name}: &str = {value:?};").unwrap();
                }
                if prefix == "ACCEPTED_G004" {
                    for (name, value) in [
                        ("LANGUAGE_PROFILE_ID", &e.language_profile_id),
                        ("DECODER_ID", &e.decoder_id),
                        ("NORMALIZATION_ID", &e.normalization_id),
                    ] {
                        writeln!(out, "/// Catalog-generated released identity.\npub const {prefix}_{name}: &str = {value:?};").unwrap();
                    }
                    writeln!(out, "/// Catalog-generated vocabulary count.\npub const {prefix}_VOCABULARY_ENTRIES: u32 = {};", e.vocabulary.entries).unwrap();
                }
            }
        }
        for (index, resource) in catalog.resources.iter().enumerate() {
            let accepted_use = catalog
                .profiles
                .iter()
                .any(|p| p.status == "accepted" && uses_resource(&p.execution, &resource.id));
            if !accepted_use {
                out.push_str("#[cfg(feature = \"model-candidates\")]\n");
            }
            writeln!(out, "static RESOURCE_{index}: ResourceDescriptor = ResourceDescriptor {{ id: {:?}, relative_path: {:?}, byte_len: {}, sha256: {:?}, source_url: {:?}, source_revision: {:?}, license: {:?}, notice: {:?} }};",
                resource.id, resource.relative_path, resource.byte_len, digest(&resource.sha256)?, resource.source_url, resource.source_revision, resource.license, resource.notice).unwrap();
        }
        for (status, table) in [
            ("accepted", "ACCEPTED_PROFILES"),
            ("candidate", "CANDIDATE_PROFILES"),
        ] {
            if status == "candidate" {
                out.push_str("#[cfg(feature = \"model-candidates\")]\n");
            }
            writeln!(out, "static {table}: &[OcrProfileDescriptor] = &[").unwrap();
            for profile in &catalog.profiles {
                if profile.status == status {
                    self.emit_profile(
                        &mut out,
                        profile,
                        digest(&digests[&profile.execution.profile_id])?,
                    )?;
                }
            }
            out.push_str("];\n");
        }
        out.push_str("static CLOSED_PROFILES: &[(&str, &str, bool)] = &[\n");
        for profile in &catalog.profiles {
            writeln!(
                out,
                "({:?}, {:?}, {}),",
                profile.execution.model_id,
                profile.execution.profile_id,
                profile.execution.auxiliary.is_some()
            )
            .unwrap();
        }
        out.push_str("];\n");
        Ok(out)
    }

    fn emit_profile(
        &self,
        out: &mut String,
        profile: &Profile,
        execution_sha256: [u8; 32],
    ) -> Result<(), String> {
        let e = &profile.execution;
        out.push_str("OcrProfileDescriptor {\n");
        for (name, value) in [
            ("model_id", &e.model_id),
            ("model_version", &e.model_version),
            ("profile_id", &e.profile_id),
            ("language_profile_id", &e.language_profile_id),
            ("preprocessing_id", &e.preprocessing_id),
            ("decoder_id", &e.decoder_id),
            ("normalization_id", &e.normalization_id),
        ] {
            writeln!(out, "{name}: {value:?},").unwrap();
        }
        let resource_index = |id: &str| {
            self.0
                .resources
                .iter()
                .position(|r| r.id == id)
                .ok_or("missing resource index")
        };
        writeln!(
            out,
            "detector: &RESOURCE_{}, recognizer: &RESOURCE_{},",
            resource_index(&e.detector)?,
            resource_index(&e.recognizer)?
        )
        .unwrap();
        match &e.auxiliary {
            Some(id) => {
                writeln!(out, "auxiliary: Some(&RESOURCE_{}),", resource_index(id)?).unwrap()
            }
            None => out.push_str("auxiliary: None,\n"),
        }
        for (name, graph) in [
            ("detector_graph", &e.detector_graph),
            ("recognizer_graph", &e.recognizer_graph),
        ] {
            writeln!(out, "{name}: GraphDescriptor {{").unwrap();
            for (name, tensor) in [("input", &graph.input), ("output", &graph.output)] {
                writeln!(
                    out,
                    "{name}: TensorDescriptor {{ name: {:?}, shape: &{:?}, axis_limits: &{:?} }},",
                    tensor.name, tensor.shape, tensor.axis_limits
                )
                .unwrap();
            }
            out.push_str("},\n");
        }
        let d = &e.detection;
        let resize = match d.resize.as_str() {
            "legacy-short-side" => "LegacyShortSide",
            "bounded-rectangular" => "BoundedRectangular",
            _ => "MaxSide",
        };
        let normalization = if d.normalization == "legacy-divide" {
            "LegacyDivide"
        } else {
            "MultiplyMeanStd"
        };
        writeln!(out, "detection: DetectorDescriptor {{ resize: DetectorResize::{resize}, side: {}, multiple: {}, minimum_dimension: {}, max_width: {:?}, max_height: {:?}, max_tensor_bytes: {:?}, normalization: PixelNormalization::{normalization}, mean: {:?}, std: {:?}, threshold: {:?}, box_threshold: {:?}, unclip_ratio: {:?}, min_side: {:?}, sort_line_threshold: {:?}, max_candidates: {} }},",
            d.side, d.multiple, d.minimum_dimension, d.max_width, d.max_height, d.max_tensor_bytes, d.mean, d.std, d.threshold, d.box_threshold, d.unclip_ratio, d.min_side, d.sort_line_threshold, d.max_candidates).unwrap();
        let r = &e.recognition;
        writeln!(out, "recognition: RecognitionDescriptor {{ height: {}, nominal_width: {}, max_width: {}, batch: {} }},", r.height, r.nominal_width, r.max_width, r.batch).unwrap();
        let v = &e.vocabulary;
        let source = if v.source == "onnx-metadata" {
            "OnnxMetadata"
        } else {
            "PaddleYaml"
        };
        writeln!(out, "vocabulary: VocabularyDescriptor {{ source: VocabularySource::{source}, entries: {}, sha256: {:?}, classes: {} }},", v.entries, digest(&v.sha256)?, v.classes).unwrap();
        let l = &e.limits;
        writeln!(out, "limits: ResourceLimits {{ input_tensor_bytes: {}, output_tensor_bytes: {}, contour_bytes: {}, scanline_bytes: {}, model_bytes: {}, auxiliary_bytes: {} }},", l.input_tensor_bytes, l.output_tensor_bytes, l.contour_bytes, l.scanline_bytes, l.model_bytes, l.auxiliary_bytes).unwrap();
        out.push_str("applicability: &[\n");
        for a in &e.applicability {
            writeln!(out, "ProfileApplicability {{ target: {:?}, provider: {:?}, runtime_version: {:?}, runtime_api: {} }},", a.target, a.provider, a.runtime_version, a.runtime_api).unwrap();
        }
        writeln!(
            out,
            "], execution_sha256: {:?}, display_name: {:?}, variant: {:?}, benchmarks: &[",
            execution_sha256, profile.presentation.display_name, profile.presentation.variant
        )
        .unwrap();
        for b in &profile.presentation.benchmarks {
            let code = match b.code.as_str() {
                "reference" => "Reference",
                "fast" => "Fast",
                "balanced" => "Balanced",
                _ => "Unrated",
            };
            writeln!(out, "BenchmarkSummary {{ target: {:?}, provider: {:?}, code: BenchmarkCode::{code}, criteria: {:?}, report: {:?} }},", b.target, b.provider, b.criteria, b.report).unwrap();
        }
        out.push_str("] },\n");
        Ok(())
    }
}

fn uses_resource(e: &Execution, id: &str) -> bool {
    e.detector == id || e.recognizer == id || e.auxiliary.as_deref() == Some(id)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

#[cfg(test)]
#[path = "catalog_build_tests.rs"]
mod tests;
