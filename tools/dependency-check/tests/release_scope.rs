//! Mutation tests for the v0.4.0 source-release boundary.

use std::collections::BTreeMap;

use mado_pilot_dependency_check::release::{
    C_HEADER_FILE, CMAKE_PROJECT_FILE, CPP_HEADER_FILE, RELEASE_NOTES_FILE,
    REQUIRED_BLOB_IDENTITIES, REQUIRED_TREE_IDENTITIES, ReleaseScopeObservation, ReleaseViolation,
    TrackedEntry, validate,
};

fn compliant_observation() -> ReleaseScopeObservation {
    let paths = [
        "AGENTS.md",
        ".gitattributes",
        "CLAUDE.md",
        RELEASE_NOTES_FILE,
        CMAKE_PROJECT_FILE,
        C_HEADER_FILE,
        CPP_HEADER_FILE,
        "fixtures/assets/g-014/valid/valid-tiny.zip",
        "fixtures/assets/ocr-public-surface/models/detector.onnx",
        "fixtures/assets/ocr-public-surface/models/recognizer.onnx",
    ];
    let mut tracked_entries = paths
        .into_iter()
        .map(|path| (path.to_owned(), regular_entry(path)))
        .collect::<BTreeMap<_, _>>();
    tracked_entries.insert(
        "AGENTS.md".to_owned(),
        TrackedEntry {
            mode: "120000".to_owned(),
            object_id: "681311eb9cf453d0faddf3aacaec7357e97ba8e9".to_owned(),
            symlink_target: Some("CLAUDE.md".to_owned()),
            utf8_text: None,
        },
    );

    ReleaseScopeObservation {
        tracked_entries,
        tree_oids: REQUIRED_TREE_IDENTITIES
            .into_iter()
            .map(|(path, object_id)| (path.to_owned(), object_id.to_owned()))
            .collect(),
        release_notes: include_str!("../../../docs/releases/v0.4.0.md").to_owned(),
        cmake_project: include_str!("../../../crates/bindings/capi/CMakeLists.txt").to_owned(),
        c_header: include_str!("../../../crates/bindings/capi/include/madopilot/madopilot.h")
            .to_owned(),
        cpp_header: include_str!("../../../crates/bindings/capi/include/madopilot/madopilot.hpp")
            .to_owned(),
    }
}

fn regular_entry(path: &str) -> TrackedEntry {
    let object_id = match path {
        ".gitattributes" => REQUIRED_BLOB_IDENTITIES[0].1,
        "fixtures/assets/ocr-public-surface/models/detector.onnx" => {
            "cc6723e5145af0e74428c9056f84709dfd06661c"
        }
        "fixtures/assets/ocr-public-surface/models/recognizer.onnx" => {
            "b7c572ac50ba66d44439245d16926ef4a62baed3"
        }
        _ => "synthetic-regular-blob",
    };
    TrackedEntry {
        mode: "100644".to_owned(),
        object_id: object_id.to_owned(),
        symlink_target: None,
        utf8_text: Some(true),
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
    observation.tracked_entries.remove(RELEASE_NOTES_FILE);

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
fn weakening_any_privacy_exclusion_is_rejected() {
    for protected in [
        "captured pixels",
        "recognized text",
        "caller template/model identities",
        "input payloads",
        "credentials",
        "raw native identifiers",
        "unrelated process/window inventories",
        "local paths",
    ] {
        let mut observation = compliant_observation();
        observation.release_notes = observation.release_notes.replace(protected, "payloads");

        assert!(
            has_violation(&observation, |violation| matches!(
                violation,
                ReleaseViolation::MissingReleaseFact { fact }
                    if *fact == "privacy exclusions"
            )),
            "removing `{protected}` must be rejected"
        );
    }

    let mut observation = compliant_observation();
    observation.release_notes = observation
        .release_notes
        .replace("release evidence exclude", "release evidence include");
    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::MissingReleaseFact { fact }
            if *fact == "privacy exclusions"
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
fn active_packaging_cmake_surfaces_are_rejected() {
    for (surface, mutation) in [
        ("install", "install(TARGETS MadoPilot)\n"),
        ("export", "export(TARGETS MadoPilot)\n"),
        ("STATIC", "add_library(MadoPilot STATIC imported.a)\n"),
        ("add_library", "add_library(extra source.c)\n"),
        ("include", "include(cmake/package.cmake)\n"),
        (
            "add_subdirectory",
            "add_subdirectory(cmake/package-support)\n",
        ),
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
    for (header, mutation, expected_token) in [
        (
            C_HEADER_FILE,
            "madopilot_template_watch",
            "madopilot_template_watch",
        ),
        (CPP_HEADER_FILE, "WATCH_TEMPLATE", "watch_template"),
    ] {
        let mut observation = compliant_observation();
        if header == C_HEADER_FILE {
            observation.c_header.push_str(mutation);
        } else {
            observation.cpp_header.push_str(mutation);
        }

        assert!(
            has_violation(&observation, |violation| matches!(
                violation,
                ReleaseViolation::ForeignWatcherSurface {
                    header: observed_header,
                    token: observed_token,
                } if *observed_header == header && *observed_token == expected_token
            )),
            "watcher token `{mutation}` in `{header}` must be rejected"
        );
    }
}

#[test]
fn private_generated_and_binary_release_inputs_are_rejected() {
    for path in [
        "rasen/changes/private.md",
        ".rasen/changes/run.json",
        "local_docs/private-fixture.md",
        "crates/mado-pilot/target/release/madopilot",
        "tools/debug/report.json",
        ".idea/workspace.xml",
        "qualification-output/native.json",
        "vendor/onnxruntime.dll",
        "vendor/libonnxruntime.1.29.0.dylib",
        "docs/benchmarks/private-capture.png",
        "vendor/onnxruntime.bin",
        "docs/.gitattributes",
        "models/production.onnx",
        "release/madopilot.zip",
        "fixtures/assets/g-014/private.zip",
        "config/.env.production",
        "vendor/madopilot",
        "crates/bindings/capi/include/madopilot/watch.h",
    ] {
        let mut observation = compliant_observation();
        observation
            .tracked_entries
            .insert(path.to_owned(), regular_entry(path));

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

#[test]
fn changed_release_relevant_subtrees_are_rejected() {
    for (tree, _) in REQUIRED_TREE_IDENTITIES {
        let mut observation = compliant_observation();
        observation
            .tree_oids
            .insert(tree.to_owned(), "changed-tree".to_owned());

        assert!(
            has_violation(&observation, |violation| matches!(
                violation,
                ReleaseViolation::UnexpectedTreeIdentity { tree: observed }
                    if *observed == tree
            )),
            "tree drift in `{tree}` must be rejected"
        );
    }
}

#[test]
fn unsafe_tree_modes_and_symlink_targets_are_rejected() {
    for (path, entry) in [
        (
            "tools/release-helper",
            TrackedEntry {
                mode: "100755".to_owned(),
                object_id: "executable".to_owned(),
                symlink_target: None,
                utf8_text: Some(true),
            },
        ),
        (
            "vendor/module",
            TrackedEntry {
                mode: "160000".to_owned(),
                object_id: "gitlink".to_owned(),
                symlink_target: None,
                utf8_text: None,
            },
        ),
        (
            "docs/escape.md",
            TrackedEntry {
                mode: "120000".to_owned(),
                object_id: "symlink".to_owned(),
                symlink_target: Some("../../private".to_owned()),
                utf8_text: None,
            },
        ),
    ] {
        let mut observation = compliant_observation();
        observation.tracked_entries.insert(path.to_owned(), entry);
        assert!(
            has_violation(&observation, |violation| matches!(
                violation,
                ReleaseViolation::ForbiddenReleaseInput { path: observed, .. }
                    if observed == path
            )),
            "tree entry `{path}` must be rejected"
        );
    }

    let mut observation = compliant_observation();
    observation
        .tracked_entries
        .get_mut("AGENTS.md")
        .expect("compliant observation has the reviewed symlink")
        .symlink_target = Some("../CLAUDE.md".to_owned());
    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::ForbiddenReleaseInput { path, .. }
            if path == "AGENTS.md"
    )));
}

#[test]
fn changed_approved_model_blob_is_rejected() {
    let mut observation = compliant_observation();
    observation
        .tracked_entries
        .get_mut("fixtures/assets/ocr-public-surface/models/detector.onnx")
        .expect("compliant observation has the detector fixture")
        .object_id = "changed-model".to_owned();

    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::ForbiddenReleaseInput { path, .. }
            if path == "fixtures/assets/ocr-public-surface/models/detector.onnx"
    )));
}

#[test]
fn changed_archive_attributes_are_rejected() {
    let mut observation = compliant_observation();
    observation
        .tracked_entries
        .get_mut(".gitattributes")
        .expect("compliant observation has archive attributes")
        .object_id = "changed-attributes".to_owned();

    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::ForbiddenReleaseInput { path, .. }
            if path == ".gitattributes"
    )));
}

#[test]
fn renamed_non_utf8_payload_is_rejected() {
    let path = "vendor/runtime.dat";
    let mut observation = compliant_observation();
    let mut entry = regular_entry(path);
    entry.utf8_text = Some(false);
    observation.tracked_entries.insert(path.to_owned(), entry);

    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::ForbiddenReleaseInput { path: observed, .. }
            if observed == path
    )));
}

#[test]
fn changed_approved_executable_is_rejected() {
    let path = "docs/evidence/g-004/evaluate.py";
    let mut observation = compliant_observation();
    observation.tracked_entries.insert(
        path.to_owned(),
        TrackedEntry {
            mode: "100755".to_owned(),
            object_id: "d5fae53a3e614aacbb608523dc28603a6c8a3995".to_owned(),
            symlink_target: None,
            utf8_text: None,
        },
    );
    assert_eq!(validate(&observation), []);

    observation
        .tracked_entries
        .get_mut(path)
        .expect("approved executable is present")
        .object_id = "changed-executable".to_owned();
    assert!(has_violation(&observation, |violation| matches!(
        violation,
        ReleaseViolation::ForbiddenReleaseInput { path: observed, .. }
            if observed == path
    )));
}
