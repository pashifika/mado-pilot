use super::{ValidatedCatalog, validate};
use serde_json::{Value, json};

const SOURCE: &str = include_str!("resources/model-catalog.json");
const CANDIDATE: &str = "ppocrv6-tiny-det-small-rec-max736-v1";

fn source() -> Value {
    serde_json::from_str(SOURCE).unwrap()
}

fn validated(value: &Value) -> Result<ValidatedCatalog, String> {
    validate(&serde_json::to_string(value).unwrap())
}

#[test]
fn unknown_missing_duplicate_and_ambiguous_contracts_are_refused() {
    let cases = [
        ("/schema_version", json!(2)),
        ("/profiles/2/status", json!("supported")),
        ("/profiles/2/execution/detector", json!("missing-resource")),
        ("/profiles/2/execution/auxiliary", Value::Null),
        (
            "/profiles/0/execution/auxiliary",
            json!("paddle-v6-small-vocabulary"),
        ),
        ("/resources/4/kind", json!("model")),
        ("/resources/0/byte_len", json!(0)),
        ("/resources/4/byte_len", json!(1048577)),
        ("/resources/0/sha256", json!("00".repeat(32))),
        ("/resources/0/source_revision", json!("main")),
        (
            "/resources/0/source_url",
            json!("https://huggingface.co/SWHL/RapidOCR/resolve/main/model.onnx"),
        ),
        ("/resources/0/license", json!("unreviewed")),
        ("/resources/0/notice", json!("")),
        (
            "/profiles/2/execution/detection/algorithm",
            json!("unknown"),
        ),
        (
            "/profiles/2/execution/detection/std",
            json!([0.229, 0, 0.225]),
        ),
        ("/profiles/2/execution/detection/threshold", json!(1.1)),
        ("/profiles/2/execution/detection/side", json!(0)),
        (
            "/profiles/2/execution/detection/minimum_dimension",
            json!(0),
        ),
        (
            "/profiles/2/execution/detection/max_tensor_bytes",
            json!(6500351),
        ),
        (
            "/profiles/0/execution/detection/mean",
            json!([0.1, 0.5, 0.5]),
        ),
        ("/profiles/2/execution/recognition/batch", json!(7)),
        (
            "/profiles/2/execution/detector_graph/input/dtype",
            json!("f16"),
        ),
        (
            "/profiles/2/execution/detector_graph/input/shape",
            json!([-1, 3, -1]),
        ),
        (
            "/profiles/2/execution/detector_graph/input/axis_limits",
            json!([1, 3, 0, 736]),
        ),
        (
            "/profiles/2/execution/recognizer_graph/output/shape",
            json!([-1, -1, 18709]),
        ),
        ("/profiles/2/execution/vocabulary/classes", json!(18709)),
        (
            "/profiles/2/execution/vocabulary/source",
            json!("automatic"),
        ),
        ("/profiles/2/execution/limits/output_tensor_bytes", json!(0)),
        (
            "/profiles/2/execution/applicability/0/runtime_version",
            json!("latest"),
        ),
        (
            "/profiles/2/execution/applicability/0/provider",
            json!("cuda"),
        ),
        ("/default_profile", json!(CANDIDATE)),
    ];
    for (pointer, value) in cases {
        let mut catalog = source();
        *catalog.pointer_mut(pointer).unwrap() = value;
        assert!(validated(&catalog).is_err(), "accepted malformed {pointer}");
    }
    let mut catalog = source();
    catalog["profiles"][2]["execution"]["unexpected"] = json!(true);
    assert!(validated(&catalog).is_err());
    catalog = source();
    catalog["profiles"][0]["execution"]
        .as_object_mut()
        .unwrap()
        .remove("auxiliary");
    assert!(
        validated(&catalog).is_err(),
        "explicit nullable fields are required"
    );
    for pointer in ["/profiles", "/resources"] {
        let mut catalog = source();
        let list = catalog
            .pointer_mut(pointer)
            .unwrap()
            .as_array_mut()
            .unwrap();
        list.push(list[0].clone());
        assert!(validated(&catalog).is_err(), "duplicate {pointer}");
    }
    let duplicated = SOURCE.replacen(
        "\"schema_version\": 1",
        "\"schema_version\":1,\"schema_version\":1",
        1,
    );
    assert!(validate(&duplicated).is_err());
    let nested = SOURCE.replacen("\"side\": 736", "\"side\":736,\"s\\u0069de\":736", 1);
    assert!(validate(&nested).is_err());
    let nonfinite = SOURCE.replacen("\"threshold\": 0.3", "\"threshold\": 1e400", 1);
    assert!(validate(&nonfinite).is_err());
}

#[test]
fn resource_paths_are_portable_and_collision_free() {
    for path in [
        "/absolute",
        "../model",
        "a/../model",
        "a//model",
        "C:/model",
        "a\\model",
        "a/./model",
        "a/NUL.onnx",
        "a/COM1",
        "a/trailing.",
        "a/model:stream",
        "a/%2e%2e/model",
    ] {
        let mut catalog = source();
        catalog["resources"][0]["relative_path"] = json!(path);
        assert!(validated(&catalog).is_err(), "accepted unsafe path {path}");
    }
    let mut catalog = source();
    catalog["resources"][1]["relative_path"] = json!(
        catalog["resources"][0]["relative_path"]
            .as_str()
            .unwrap()
            .to_ascii_uppercase()
    );
    assert!(validated(&catalog).is_err());
}

fn attach_evidence(catalog: &mut Value, index: usize, code: &str) {
    let profile = &mut catalog["profiles"][index];
    profile["status"] = json!("accepted");
    profile["presentation"]["benchmarks"] = Value::Array(profile["execution"]["applicability"].as_array().unwrap().iter().map(|a| {
        json!({"target": a["target"], "provider": a["provider"], "code": code,
            "criteria": "fixtures/ocr/criteria.json", "report": format!("docs/evidence/{}.json", a["target"].as_str().unwrap())})
    }).collect());
}

#[test]
fn new_admission_requires_complete_scoped_evidence_not_unrated() {
    let mut catalog = source();
    catalog["profiles"][2]["status"] = json!("accepted");
    assert!(validated(&catalog).is_err());
    for code in ["reference", "fast", "balanced"] {
        attach_evidence(&mut catalog, 2, code);
        assert!(
            validated(&catalog).is_ok(),
            "complete schema binding for {code}"
        );
    }
    for (pointer, value) in [
        (
            "/profiles/2/presentation/benchmarks/0/code",
            json!("unrated"),
        ),
        ("/profiles/2/presentation/benchmarks/0/report", Value::Null),
        ("/profiles/2/presentation/benchmarks/0/criteria", json!("")),
        (
            "/profiles/2/presentation/benchmarks/0/target",
            json!("unqualified-target"),
        ),
    ] {
        let mut incomplete = catalog.clone();
        *incomplete.pointer_mut(pointer).unwrap() = value;
        assert!(
            validated(&incomplete).is_err(),
            "accepted incomplete {pointer}"
        );
    }
    catalog["profiles"][2]["presentation"]["benchmarks"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(validated(&catalog).is_err(), "one target is not admission");
    catalog = source();
    attach_evidence(&mut catalog, 2, "balanced");
    catalog["profiles"][2]["status"] = json!("candidate");
    assert!(
        validated(&catalog).is_err(),
        "candidate must not publish a comparative verdict"
    );
}

#[test]
fn canonical_execution_digest_excludes_presentation_and_evidence_only() {
    let original = validate(SOURCE).unwrap().execution_digests().unwrap();
    let mut catalog = source();
    catalog["profiles"][0]["presentation"]["display_name"] = json!("Localized name");
    catalog["profiles"][0]["presentation"]["variant"] = json!("Localized variant");
    attach_evidence(&mut catalog, 0, "reference");
    catalog["resources"].as_array_mut().unwrap().reverse();
    catalog["profiles"][0]["execution"]["applicability"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_eq!(
        validated(&catalog).unwrap().execution_digests().unwrap(),
        original
    );
    catalog = source();
    catalog["profiles"][2]["execution"]["detection"]["threshold"] = json!(0.31);
    let changed = validated(&catalog).unwrap().execution_digests().unwrap();
    assert_ne!(changed[CANDIDATE], original[CANDIDATE]);
    let default = mado_pilot_ocr::DEFAULT_OCR_PROFILE_ID;
    assert_eq!(changed[default], original[default]);
    catalog = source();
    catalog["resources"][4]["sha256"] = json!("01".repeat(32));
    let changed = validated(&catalog).unwrap().execution_digests().unwrap();
    assert_ne!(changed[CANDIDATE], original[CANDIDATE]);
    assert_eq!(changed[default], original[default]);
}

#[test]
fn backend_unsupported_candidate_limit_is_refused_at_catalog_admission() {
    let mut catalog = source();
    catalog["profiles"][2]["execution"]["detection"]["max_candidates"] = json!(500);
    assert!(validated(&catalog).is_err());
}

#[test]
fn backend_unsupported_vocabulary_is_refused_even_with_consistent_graph_bounds() {
    let mut catalog = source();
    let execution = &mut catalog["profiles"][2]["execution"];
    execution["vocabulary"]["entries"] = json!(6623);
    execution["vocabulary"]["classes"] = json!(6625);
    execution["recognizer_graph"]["output"]["shape"] = json!([-1, -1, 6625]);
    execution["recognizer_graph"]["output"]["axis_limits"] =
        json!([6, 256 * 1024 * 1024 / 4 / 6625, 6625]);
    assert!(validated(&catalog).is_err());
}
