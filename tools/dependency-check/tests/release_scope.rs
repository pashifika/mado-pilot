//! Mutation tests for the v0.4.0 source-release boundary.

use std::collections::BTreeSet;

use mado_pilot_dependency_check::release::{
    C_HEADER_FILE, CMAKE_PROJECT_FILE, CPP_HEADER_FILE, RELEASE_NOTES_FILE,
    ReleaseScopeObservation, ReleaseViolation, validate,
};

fn compliant_observation() -> ReleaseScopeObservation {
    let tracked_paths = [
        RELEASE_NOTES_FILE,
        CMAKE_PROJECT_FILE,
        C_HEADER_FILE,
        CPP_HEADER_FILE,
        "fixtures/assets/g-014/valid/valid-tiny.zip",
        "fixtures/assets/ocr-public-surface/models/detector.onnx",
        "fixtures/assets/ocr-public-surface/models/recognizer.onnx",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();

    ReleaseScopeObservation {
        tracked_paths,
        release_notes: include_str!("../../../docs/releases/v0.4.0.md").to_owned(),
        cmake_project: "project(\n  MadoPilot\n  VERSION 0.4.0\n  LANGUAGES C CXX\n)\n".to_owned(),
        c_header: "madopilot_status_t madopilot_get_api(void);\n".to_owned(),
        cpp_header: "namespace madopilot { class Session; }\n".to_owned(),
    }
}

fn has_violation(
    observation: &ReleaseScopeObservation,
    predicate: impl Fn(&ReleaseViolation) -> bool,
) -> bool {
    validate(observation).iter().any(predicate)
}

#[test]
fn the_canonical_release_scope_is_compliant() {
    assert_eq!(validate(&compliant_observation()), []);
}

#[test]
fn an_untracked_release_body_is_rejected() {
    let mut observation = compliant_observation();
    observation.tracked_paths.remove(RELEASE_NOTES_FILE);

    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::UntrackedReleaseNotes
    )));
}

#[test]
fn removing_a_required_support_fact_is_rejected() {
    let mut observation = compliant_observation();
    observation.release_notes = observation
        .release_notes
        .replace("ScreenCaptureKit", "native capture");

    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::MissingReleaseFact { fact }
            if *fact == "macOS ScreenCaptureKit watcher"
    )));
}

#[test]
fn removing_the_unavailable_artifact_inventory_is_rejected() {
    let mut observation = compliant_observation();
    observation.release_notes = observation.release_notes.replace(
        "Crates.io packages, prebuilt or static libraries, installers, package-manager artifacts, CMake install/export metadata, pkg-config metadata, ABI-major decorated libraries, and bundled OpenCV, ONNX Runtime, OCR models, CUDA, or cuDNN are not provided.",
        "Installable artifacts are not provided.",
    );

    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::MissingReleaseFact { fact }
            if *fact == "unavailable artifact inventory"
    )));
}

#[test]
fn a_drifted_cmake_version_is_rejected() {
    let mut observation = compliant_observation();
    observation.cmake_project = observation.cmake_project.replace("0.4.0", "0.4.1");

    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::UnexpectedCmakeVersion
    )));
}

#[test]
fn install_export_and_static_cmake_surfaces_are_rejected() {
    for (surface, mutation) in [
        ("install", "install(TARGETS MadoPilot)\n"),
        ("export", "export(TARGETS MadoPilot)\n"),
        ("STATIC", "add_library(MadoPilot STATIC imported.a)\n"),
    ] {
        let mut observation = compliant_observation();
        observation.cmake_project.push_str(mutation);

        assert!(
            has_violation(&observation, |violation| matches!(
                violation,
                ReleaseViolation::ForbiddenCmakeSurface { surface: observed }
                    if *observed == surface
            )),
            "mutation `{surface}` must be rejected"
        );
    }
}

#[test]
fn a_c_or_cpp_watcher_surface_is_rejected() {
    for (header, token) in [
        (C_HEADER_FILE, "madopilot_template_watch"),
        (CPP_HEADER_FILE, "TemplateQuery"),
    ] {
        let mut observation = compliant_observation();
        if header == C_HEADER_FILE {
            observation.c_header.push_str(token);
        } else {
            observation.cpp_header.push_str(token);
        }

        assert!(
            has_violation(&observation, |violation| matches!(
                violation,
                ReleaseViolation::ForeignWatcherSurface {
                    header: observed_header,
                    token: observed_token,
                } if *observed_header == header && *observed_token == token
            )),
            "watcher token `{token}` in `{header}` must be rejected"
        );
    }
}

#[test]
fn private_generated_and_binary_release_inputs_are_rejected() {
    for path in [
        "rasen/changes/private.md",
        ".rasen/changes/run.json",
        "local_docs/private-fixture.md",
        "target/release/madopilot.dll",
        "qualification-output/native.json",
        "vendor/onnxruntime.dll",
        "models/production.onnx",
        "release/madopilot.zip",
    ] {
        let mut observation = compliant_observation();
        observation.tracked_paths.insert(path.to_owned());

        assert!(
            has_violation(&observation, |violation| matches!(
                violation,
                ReleaseViolation::ForbiddenReleaseInput { path: observed, .. }
                    if observed == path
            )),
            "tracked input `{path}` must be rejected"
        );
    }
}
