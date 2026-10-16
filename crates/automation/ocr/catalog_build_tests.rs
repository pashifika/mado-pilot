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
        ("/schema_version", json!(1)),
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
    catalog["profiles"][2]["status"] = json!("accepted");
    assert!(
        validated(&catalog).is_err(),
        "obsolete status is not a schema-v2 field"
    );
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
        "\"schema_version\": 2",
        "\"schema_version\":2,\"schema_version\":2",
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

fn attach_evidence(catalog: &mut Value, index: usize, outcome: &str) {
    let digests = validated(catalog).unwrap().execution_digests().unwrap();
    let profile = &mut catalog["profiles"][index];
    let execution_sha256 = &digests[profile["execution"]["profile_id"].as_str().unwrap()];
    profile["presentation"]["evaluations"] = json!([{
        "target": "aarch64-apple-darwin",
        "provider": "cpu",
        "criteria": "tools/ocr-models/criteria-v1.json",
        "evidence": {
            "report": "docs/benchmarks/ocr-models/test-evaluation.json",
            "measured_source_revision": "12".repeat(20),
            "execution_sha256": execution_sha256,
            "evaluator_revision": format!("sha256:{}", "34".repeat(32)),
        },
        "rules": [
            {"rule_id": "text_order", "outcome": outcome},
            {"rule_id": "lifecycle", "outcome": "passed"},
            {"rule_id": "relative_latency", "outcome": "not_run"},
            {"rule_id": "geometry", "outcome": "incomplete"},
        ],
    }]);
}

#[test]
fn independent_mixed_and_unavailable_evaluations_do_not_change_choices() {
    let mut catalog = source();
    let original = validated(&catalog).unwrap().execution_digests().unwrap();
    for outcome in ["passed", "failed", "not_run", "incomplete"] {
        attach_evidence(&mut catalog, 2, outcome);
        assert_eq!(
            validated(&catalog).unwrap().execution_digests().unwrap(),
            original,
            "a single target's outcomes cannot change model choices"
        );
    }
    let evaluations = &mut catalog["profiles"][2]["presentation"]["evaluations"];
    let mut missing = evaluations[0].clone();
    missing["target"] = json!("x86_64-pc-windows-msvc");
    missing["evidence"] = Value::Null;
    missing["rules"] = json!([
        {"rule_id": "text_order", "outcome": "not_run"},
        {"rule_id": "lifecycle", "outcome": "incomplete"},
    ]);
    evaluations.as_array_mut().unwrap().push(missing);
    assert_eq!(
        validated(&catalog).unwrap().execution_digests().unwrap(),
        original
    );
    catalog["profiles"][2]["presentation"]["evaluations"] = json!([]);
    assert_eq!(
        validated(&catalog).unwrap().execution_digests().unwrap(),
        original
    );
}

#[test]
fn observed_outcomes_require_exact_source_bound_scoped_evidence() {
    let mut catalog = source();
    attach_evidence(&mut catalog, 2, "failed");
    for (pointer, value) in [
        ("/target", json!("unknown-target")),
        ("/provider", json!("cuda")),
        ("/criteria", Value::Null),
        ("/criteria", json!("../criteria.json")),
        ("/evidence", Value::Null),
        ("/evidence/report", json!("/absolute/report.json")),
        ("/evidence/report", json!("docs/../report.json")),
        ("/evidence/report", json!("C:\\report.json")),
        ("/evidence/measured_source_revision", json!("main")),
        ("/evidence/measured_source_revision", json!("00".repeat(20))),
        ("/evidence/measured_source_revision", json!("AB".repeat(20))),
        ("/evidence/execution_sha256", json!("01".repeat(32))),
        ("/evidence/execution_sha256", json!("00".repeat(32))),
        ("/evidence/evaluator_revision", json!("HEAD")),
        ("/evidence/evaluator_revision", json!("sha256:123")),
        (
            "/evidence/evaluator_revision",
            json!(format!("sha256:{}", "00".repeat(32))),
        ),
        ("/rules", json!([])),
        ("/rules/0/rule_id", json!("unknown_rule")),
        ("/rules/0/outcome", json!("fast")),
    ] {
        let mut invalid = catalog.clone();
        *invalid["profiles"][2]["presentation"]["evaluations"][0]
            .pointer_mut(pointer)
            .unwrap() = value;
        assert!(validated(&invalid).is_err(), "invalid evaluation {pointer}");
    }
    for outcome in ["passed", "failed"] {
        let mut invalid = catalog.clone();
        let evaluation = &mut invalid["profiles"][2]["presentation"]["evaluations"][0];
        evaluation["evidence"] = Value::Null;
        evaluation["rules"] = json!([{"rule_id": "text_order", "outcome": outcome}]);
        assert!(validated(&invalid).is_err(), "unbound {outcome}");
    }
    for field in ["criteria", "evidence", "rules"] {
        let mut invalid = catalog.clone();
        invalid["profiles"][2]["presentation"]["evaluations"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(validated(&invalid).is_err(), "missing {field}");
    }
    for pointer in [
        "/profiles/2/presentation/evaluations",
        "/profiles/2/presentation/evaluations/0/rules",
    ] {
        let mut invalid = catalog.clone();
        let entries = invalid
            .pointer_mut(pointer)
            .unwrap()
            .as_array_mut()
            .unwrap();
        entries.push(entries[0].clone());
        assert!(validated(&invalid).is_err(), "duplicate {pointer}");
    }
    for pointer in [
        "/profiles/2/presentation",
        "/profiles/2/presentation/evaluations/0",
        "/profiles/2/presentation/evaluations/0/evidence",
        "/profiles/2/presentation/evaluations/0/rules/0",
    ] {
        let mut invalid = catalog.clone();
        invalid.pointer_mut(pointer).unwrap()["unknown"] = json!(true);
        assert!(validated(&invalid).is_err(), "unknown field in {pointer}");
    }
}

#[test]
fn catalog_v2_retains_the_released_execution_fingerprints() {
    let digests = validate(SOURCE).unwrap().execution_digests().unwrap();
    assert_eq!(
        digests[mado_pilot_ocr::ACCEPTED_G004_PROFILE_ID],
        "0bcd2b77c5902cfa98c1892d291ce8d802343f16eb95f99f48b76b11879461a9"
    );
    assert_eq!(
        digests[mado_pilot_ocr::ACCEPTED_BOUNDED_PROFILE_ID],
        "469bb6af5f94b92d5d095db33be0876f7ecb8a3d31665f396e7fa0ae5350446f"
    );
    assert_eq!(
        digests[CANDIDATE],
        "27f274d6ccf856771946dcc546bcf1dd267c6f193a1874fe5c386029ce122548"
    );
}

#[test]
fn canonical_execution_digest_excludes_presentation_and_evidence_only() {
    let original = validate(SOURCE).unwrap().execution_digests().unwrap();
    let mut catalog = source();
    catalog["profiles"][0]["presentation"]["display_name"] = json!("Localized name");
    catalog["profiles"][0]["presentation"]["variant"] = json!("Localized variant");
    attach_evidence(&mut catalog, 0, "failed");
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
    catalog["profiles"][2]["presentation"]["evaluations"] = json!([]);
    catalog["profiles"][2]["execution"]["detection"]["threshold"] = json!(0.31);
    let changed = validated(&catalog).unwrap().execution_digests().unwrap();
    assert_ne!(changed[CANDIDATE], original[CANDIDATE]);
    let default = mado_pilot_ocr::DEFAULT_OCR_PROFILE_ID;
    assert_eq!(changed[default], original[default]);
    catalog = source();
    catalog["profiles"][2]["presentation"]["evaluations"] = json!([]);
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
