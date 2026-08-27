//! Contract and drift tests for the frozen G-005 offline evaluator.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mado_pilot_testkit::change_detection::{
    CandidateStatus, EvaluationDecision, EvaluationErrorKind, RecordedSequenceSet, evaluate_g005,
};
use serde_json::Value;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempRepository(PathBuf);

impl TempRepository {
    fn copy_fixture() -> Self {
        let source_root = repository_root();
        let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mado-pilot-g005-{}-{nonce}", std::process::id()));
        let fixture_relative = Path::new("fixtures/change-detection/g-005");
        let source_fixture = source_root.join(fixture_relative);
        let target_fixture = root.join(fixture_relative);
        fs::create_dir_all(target_fixture.join("frames")).expect("temporary fixture directory");

        for document in ["fixture-manifest.json", "expected-rows.json"] {
            fs::copy(source_fixture.join(document), target_fixture.join(document))
                .expect("copy frozen document");
        }
        let manifest: Value = serde_json::from_slice(
            &fs::read(source_fixture.join("fixture-manifest.json")).expect("frozen manifest"),
        )
        .expect("manifest JSON");
        for frame in manifest["frames"].as_array().expect("manifest frames") {
            let path = frame["path"].as_str().expect("frame path");
            let relative = Path::new(path);
            fs::copy(source_root.join(relative), root.join(relative)).expect("copy frame bytes");
        }

        Self(root)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    fn fixture_path(&self, filename: &str) -> PathBuf {
        self.0
            .join("fixtures/change-detection/g-005")
            .join(filename)
    }

    fn mutate_manifest(&self, mutate: impl FnOnce(&mut Value)) {
        mutate_json(&self.fixture_path("fixture-manifest.json"), mutate);
    }

    fn mutate_expected(&self, mutate: impl FnOnce(&mut Value)) {
        mutate_json(&self.fixture_path("expected-rows.json"), mutate);
    }
}

impl Drop for TempRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn mutate_json(path: &Path, mutate: impl FnOnce(&mut Value)) {
    let mut value: Value =
        serde_json::from_slice(&fs::read(path).expect("read temporary JSON")).expect("parse JSON");
    mutate(&mut value);
    fs::write(
        path,
        serde_json::to_vec_pretty(&value).expect("serialize temporary JSON"),
    )
    .expect("write temporary JSON");
}

fn exact_report(
    report: &mado_pilot_testkit::change_detection::EvaluationReport,
) -> &mado_pilot_testkit::change_detection::CandidateReport {
    report
        .candidates()
        .iter()
        .find(|candidate| candidate.candidate_id() == "exact-rgba-v1")
        .expect("exact candidate")
}

#[test]
fn frozen_fixture_checksums_cover_every_component() {
    mado_pilot_testkit::fixture_checksums::verify(
        &repository_root().join("fixtures/change-detection/g-005"),
    );
}

#[test]
fn strict_loader_accepts_the_complete_frozen_matrix() {
    let sequences = RecordedSequenceSet::load(&repository_root()).expect("frozen sequence set");

    assert_eq!(sequences.fixture_set(), "g-005-v1");
    assert_eq!(sequences.transition_count(), 9);
}

#[test]
fn strict_loader_rejects_unknown_manifest_fields() {
    let repository = TempRepository::copy_fixture();
    repository.mutate_manifest(|manifest| manifest["unexpected"] = Value::Bool(true));

    let error = RecordedSequenceSet::load(repository.root()).expect_err("unknown field");
    assert_eq!(error.kind(), EvaluationErrorKind::InvalidJson);
}

#[test]
fn strict_loader_rejects_digest_mismatch_without_exposing_content() {
    let repository = TempRepository::copy_fixture();
    let frame = repository.fixture_path("frames/low-area-1.rgba");
    let mut bytes = fs::read(&frame).expect("temporary frame");
    bytes[0] ^= 0xff;
    fs::write(frame, bytes).expect("mutate temporary frame");

    let error = RecordedSequenceSet::load(repository.root()).expect_err("digest mismatch");
    assert_eq!(error.kind(), EvaluationErrorKind::DigestMismatch);
    assert_eq!(error.component_id(), Some("low-area-1"));
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(".rgba"));
    assert!(!diagnostic.contains("fixtures/"));
    assert!(!diagnostic.contains("sha256"));
}

#[test]
fn strict_loader_rejects_duplicate_and_reordered_frames() {
    let duplicate = TempRepository::copy_fixture();
    duplicate.mutate_manifest(|manifest| {
        manifest["frames"][1]["id"] = manifest["frames"][0]["id"].clone();
    });
    let error = RecordedSequenceSet::load(duplicate.root()).expect_err("duplicate frame");
    assert_eq!(error.kind(), EvaluationErrorKind::DuplicateComponent);

    let reordered = TempRepository::copy_fixture();
    reordered.mutate_manifest(|manifest| {
        manifest["frames"]
            .as_array_mut()
            .expect("frames")
            .swap(0, 1);
    });
    let error = RecordedSequenceSet::load(reordered.root()).expect_err("reordered frame");
    assert_eq!(error.kind(), EvaluationErrorKind::InvalidFrameOrder);
}

#[test]
fn strict_loader_rejects_missing_duplicate_and_reordered_expected_rows() {
    let missing = TempRepository::copy_fixture();
    missing.mutate_expected(|expected| {
        expected["rows"].as_array_mut().expect("rows").pop();
    });
    let error = RecordedSequenceSet::load(missing.root()).expect_err("missing expected row");
    assert_eq!(error.kind(), EvaluationErrorKind::InvalidComponentLength);

    let duplicate = TempRepository::copy_fixture();
    duplicate.mutate_expected(|expected| {
        let rows = expected["rows"].as_array_mut().expect("rows");
        rows[1] = rows[0].clone();
    });
    let error = RecordedSequenceSet::load(duplicate.root()).expect_err("duplicate expected row");
    assert_eq!(error.kind(), EvaluationErrorKind::DuplicateComponent);

    let reordered = TempRepository::copy_fixture();
    reordered.mutate_expected(|expected| {
        expected["rows"].as_array_mut().expect("rows").swap(0, 1);
    });
    let error = RecordedSequenceSet::load(reordered.root()).expect_err("reordered expected row");
    assert_eq!(error.kind(), EvaluationErrorKind::InvalidExpectedRow);
}

#[test]
fn strict_loader_rejects_roi_overflow_and_noncanonical_paths() {
    let overflow = TempRepository::copy_fixture();
    overflow.mutate_manifest(|manifest| {
        manifest["sequences"][0]["roi"]["x"] = Value::from(u64::MAX);
        manifest["sequences"][0]["roi"]["width"] = Value::from(2);
    });
    let error = RecordedSequenceSet::load(overflow.root()).expect_err("ROI overflow");
    assert_eq!(error.kind(), EvaluationErrorKind::ArithmeticOverflow);

    let path = TempRepository::copy_fixture();
    path.mutate_manifest(|manifest| {
        manifest["frames"][0]["path"] = Value::String("../../private.rgba".to_owned());
    });
    let error = RecordedSequenceSet::load(path.root()).expect_err("noncanonical path");
    assert_eq!(error.kind(), EvaluationErrorKind::InvalidFrameReference);
    assert!(!format!("{error:?} {error}").contains("private.rgba"));
}

#[test]
fn strict_loader_bounds_transition_ids_before_content_redacted_errors() {
    let repository = TempRepository::copy_fixture();
    let long_suffix = "9".repeat(4096);
    repository.mutate_expected(|expected| {
        expected["rows"][0]["transition_id"] = Value::String(format!("no-change/{long_suffix}"));
    });

    let error = RecordedSequenceSet::load(repository.root()).expect_err("oversized transition id");
    assert_eq!(error.kind(), EvaluationErrorKind::InvalidIdentifier);
    assert_eq!(error.component_id(), None);
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(&long_suffix));

    let noncanonical = TempRepository::copy_fixture();
    noncanonical.mutate_expected(|expected| {
        expected["rows"][0]["transition_id"] = Value::String("no-change/00".to_owned());
    });
    let error =
        RecordedSequenceSet::load(noncanonical.root()).expect_err("noncanonical transition index");
    assert_eq!(error.kind(), EvaluationErrorKind::InvalidIdentifier);
    assert_eq!(error.component_id(), None);
}

#[test]
fn evaluator_clips_roi_and_retains_the_one_pixel_must_detect_gate() {
    let sequences = RecordedSequenceSet::load(&repository_root()).expect("frozen sequence set");
    let report = evaluate_g005(&sequences);
    let exact = exact_report(&report);

    assert_eq!(exact.status(), CandidateStatus::Passed);
    assert_eq!(exact.aggregates().false_skip_count(), 0);
    assert_eq!(
        exact.transitions()[2].decision(),
        EvaluationDecision::AnalysisRequired
    );
}

#[test]
fn repeated_pixels_skip_but_geometry_and_epoch_discontinuities_force_analysis() {
    let sequences = RecordedSequenceSet::load(&repository_root()).expect("frozen sequence set");
    let report = evaluate_g005(&sequences);

    for candidate in report.candidates() {
        assert_eq!(candidate.transitions().len(), 9);
        assert_eq!(
            candidate.transitions()[6].decision(),
            EvaluationDecision::Unchanged
        );
        assert_eq!(
            candidate.transitions()[7].decision(),
            EvaluationDecision::AnalysisRequired
        );
        assert_eq!(
            candidate.transitions()[8].decision(),
            EvaluationDecision::AnalysisRequired
        );
    }
}

#[test]
fn candidate_comparison_rejects_false_skips_and_selects_exact_rgba() {
    let sequences = RecordedSequenceSet::load(&repository_root()).expect("frozen sequence set");
    let report = evaluate_g005(&sequences);

    assert_eq!(report.selected_policy_id(), "exact-rgba-v1");
    assert_eq!(report.candidates().len(), 7);
    assert_eq!(report.candidates()[0].status(), CandidateStatus::Passed);
    for rejected in &report.candidates()[1..] {
        assert_eq!(rejected.status(), CandidateStatus::Rejected);
        assert!(rejected.aggregates().false_skip_count() > 0);
    }
}

#[test]
fn aggregate_output_is_complete_private_and_byte_stable() {
    let sequences = RecordedSequenceSet::load(&repository_root()).expect("frozen sequence set");
    let first = evaluate_g005(&sequences)
        .to_canonical_json()
        .expect("canonical report");
    let second = evaluate_g005(&sequences)
        .to_canonical_json()
        .expect("canonical report");

    assert_eq!(first, second);
    assert!(first.ends_with(b"\n"));
    let report: Value = serde_json::from_slice(&first).expect("report JSON");
    assert_eq!(report["schema"], "mado-pilot-change-evaluation-report-v2");
    assert_eq!(
        report["candidates"].as_array().expect("candidates").len(),
        7
    );
    for candidate in report["candidates"].as_array().expect("candidates") {
        assert_eq!(
            candidate["transitions"]
                .as_array()
                .expect("transitions")
                .len(),
            9
        );
    }

    let text = String::from_utf8(first).expect("UTF-8 report");
    for forbidden in [
        "frames/",
        ".rgba",
        "pixel_bytes",
        "frame_sha256",
        "credential",
        "desktop",
        "window_title",
        "native_payload",
        "decoder_error",
    ] {
        assert!(!text.contains(forbidden), "report exposed {forbidden}");
    }
}

#[test]
fn accepted_report_and_runtime_descriptor_remain_exactly_aligned() {
    let root = repository_root();
    let sequences = RecordedSequenceSet::load(&root).expect("frozen sequence set");
    let actual = evaluate_g005(&sequences)
        .to_canonical_json()
        .expect("canonical report");
    let accepted =
        fs::read(root.join("docs/evidence/g-005/accepted-report.json")).expect("accepted report");
    assert_eq!(actual, accepted);

    let report: Value = serde_json::from_slice(&accepted).expect("accepted report JSON");
    let descriptor = mado_pilot_vision::DEFAULT_CHANGE_DETECTION_DESCRIPTOR;
    assert_eq!(report["selected_policy_id"], descriptor.policy_id());
    assert_eq!(
        report["authority"]["unchanged_may_skip_routine_analysis"],
        descriptor.unchanged_may_skip_routine_analysis()
    );
    assert_eq!(
        report["authority"]["unchanged_confirms_presence"],
        descriptor.unchanged_confirms_presence()
    );
    assert_eq!(
        report["authority"]["unchanged_advances_consecutive_stability"],
        descriptor.unchanged_advances_consecutive_stability()
    );
    assert_eq!(
        report["authority"]["unchanged_creates_duration_stability"],
        descriptor.unchanged_creates_duration_stability()
    );
    assert_eq!(
        report["authority"]["unchanged_crosses_incompatible_identity_or_geometry"],
        descriptor.unchanged_crosses_incompatible_identity_or_geometry()
    );
}

#[test]
fn adr_candidate_table_and_frozen_identities_match_the_accepted_report() {
    let root = repository_root();
    let report: Value = serde_json::from_slice(
        &fs::read(root.join("docs/evidence/g-005/accepted-report.json")).expect("accepted report"),
    )
    .expect("accepted report JSON");
    let adr =
        fs::read_to_string(root.join("docs/adr/0050-change-detection-default.md")).expect("ADR");

    for (label, report_field) in [
        ("fixture manifest", "manifest_sha256"),
        ("expected rows", "expected_rows_sha256"),
        (
            "security-remediated evaluator source",
            "evaluator_source_sha256",
        ),
        ("canonical candidate plan", "candidate_plan_sha256"),
    ] {
        let digest = report[report_field].as_str().expect("report digest");
        assert!(
            adr.contains(&format!("| {label} | `{digest}` |")),
            "ADR identity drift for {label}"
        );
    }

    for candidate in report["candidates"].as_array().expect("candidate reports") {
        let id = candidate["candidate_id"].as_str().expect("candidate id");
        let aggregates = &candidate["aggregates"];
        let decision = if candidate["status"] == "passed" {
            "pass / selected"
        } else {
            "reject"
        };
        let table_row = format!(
            "| `{id}` | {} | {} | {} | {} | {decision} |",
            aggregates["false_skip_count"],
            aggregates["admitted_analysis_count"],
            aggregates["skipped_analysis_count"],
            aggregates["inspected_pixel_count"],
        );
        assert!(adr.contains(&table_row), "ADR result drift for {id}");
    }

    assert!(adr.contains(
        "An unchanged result authorizes only skipping routine visual analysis for that compatible transition."
    ));
    assert!(adr.contains(
        "any future false skip, fixture/report/policy drift, unsupported descriptor requirement"
    ));
}
