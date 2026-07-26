//! Deterministic architecture-rule tests.
//!
//! Every case builds a synthetic normalized graph, so results depend only on the
//! supplied package inventory and edges. Cargo is never invoked.

use mado_pilot_dependency_check::graph::{
    ALLOWED_DEPENDENCIES, ASSETS, BACKEND_ONNX, BACKEND_OPENCV, CAPI, CAPTURE, CORE,
    DEPENDENCY_CHECK, FACADE, INPUT, OCR, ObservedEdge, ObservedPackage, PLATFORM_MACOS,
    PLATFORM_WINDOWS, PackageGraph, REQUIRED_PACKAGES, RUNTIME, TESTKIT, VISION, Violation,
    validate,
};

fn baseline_packages() -> Vec<ObservedPackage> {
    REQUIRED_PACKAGES
        .iter()
        .map(|package| ObservedPackage::new(package.name, package.directory))
        .collect()
}

fn graph_with(edges: Vec<ObservedEdge>) -> PackageGraph {
    PackageGraph::new(baseline_packages(), edges)
}

fn forbidden_edges(violations: &[Violation]) -> Vec<(&str, &str)> {
    violations
        .iter()
        .filter_map(|violation| match violation {
            Violation::ForbiddenDependency { from, to, .. } => Some((from.as_str(), to.as_str())),
            _ => None,
        })
        .collect()
}

#[test]
fn workspace_without_any_madopilot_edge_is_accepted() {
    // Phase 0 declares no product edge at all; omitted future edges are valid.
    assert_eq!(validate(&graph_with(Vec::new())), Vec::new());
}

#[test]
fn every_allowed_adjacency_group_is_accepted() {
    let edges = ALLOWED_DEPENDENCIES
        .iter()
        .flat_map(|(source, destinations)| {
            destinations
                .iter()
                .map(move |destination| ObservedEdge::production(*source, *destination))
        })
        .collect();

    assert_eq!(validate(&graph_with(edges)), Vec::new());
}

#[test]
fn documented_cross_contract_edges_are_accepted() {
    let edges = vec![
        ObservedEdge::production(VISION, CAPTURE),
        ObservedEdge::production(OCR, CAPTURE),
        ObservedEdge::production(OCR, VISION),
        ObservedEdge::production(ASSETS, VISION),
        ObservedEdge::production(ASSETS, OCR),
    ];

    assert_eq!(validate(&graph_with(edges)), Vec::new());
}

#[test]
fn reverse_cross_contract_edges_are_rejected() {
    let edges = vec![
        ObservedEdge::production(CAPTURE, VISION),
        ObservedEdge::production(CAPTURE, OCR),
        ObservedEdge::production(VISION, OCR),
        ObservedEdge::production(OCR, ASSETS),
    ];

    let violations = validate(&graph_with(edges));

    assert_eq!(
        forbidden_edges(&violations),
        vec![
            (CAPTURE, OCR),
            (CAPTURE, VISION),
            (OCR, ASSETS),
            (VISION, OCR),
        ]
    );
}

#[test]
fn core_may_not_depend_on_any_product_package() {
    for destination in [CAPTURE, INPUT, VISION, OCR, ASSETS, RUNTIME, FACADE] {
        let violations = validate(&graph_with(vec![ObservedEdge::production(
            CORE,
            destination,
        )]));
        assert_eq!(
            forbidden_edges(&violations),
            vec![(CORE, destination)],
            "core must not depend on {destination}"
        );
    }
}

#[test]
fn contract_package_may_not_depend_on_an_adapter() {
    for destination in [
        PLATFORM_WINDOWS,
        PLATFORM_MACOS,
        BACKEND_OPENCV,
        BACKEND_ONNX,
    ] {
        let violations = validate(&graph_with(vec![ObservedEdge::production(
            CAPTURE,
            destination,
        )]));
        assert_eq!(
            forbidden_edges(&violations),
            vec![(CAPTURE, destination)],
            "capture must not depend on {destination}"
        );
    }
}

#[test]
fn runtime_dependency_on_a_concrete_adapter_is_rejected() {
    for destination in [
        PLATFORM_WINDOWS,
        PLATFORM_MACOS,
        BACKEND_OPENCV,
        BACKEND_ONNX,
    ] {
        let violations = validate(&graph_with(vec![ObservedEdge::production(
            RUNTIME,
            destination,
        )]));
        assert_eq!(
            forbidden_edges(&violations),
            vec![(RUNTIME, destination)],
            "runtime must not know {destination}"
        );
    }
}

#[test]
fn c_abi_dependency_bypassing_the_facade_is_rejected() {
    for destination in [
        RUNTIME,
        CORE,
        CAPTURE,
        PLATFORM_WINDOWS,
        PLATFORM_MACOS,
        BACKEND_OPENCV,
        BACKEND_ONNX,
    ] {
        let violations = validate(&graph_with(vec![ObservedEdge::production(
            CAPI,
            destination,
        )]));
        assert_eq!(
            forbidden_edges(&violations),
            vec![(CAPI, destination)],
            "the C ABI must reach {destination} only through the facade"
        );
    }
}

#[test]
fn facade_dependency_on_the_c_abi_is_rejected() {
    let violations = validate(&graph_with(vec![ObservedEdge::production(FACADE, CAPI)]));

    assert_eq!(forbidden_edges(&violations), vec![(FACADE, CAPI)]);
}

#[test]
fn production_dependency_on_test_support_is_rejected() {
    for source in [CORE, CAPTURE, RUNTIME, FACADE, CAPI] {
        let violations = validate(&graph_with(vec![ObservedEdge::production(source, TESTKIT)]));
        assert_eq!(
            violations,
            vec![Violation::TestSupportInProduction {
                from: source.to_owned()
            }],
            "{source} must not ship a dependency on test support"
        );
    }
}

#[test]
fn development_dependency_on_test_support_is_accepted() {
    let edges = vec![
        ObservedEdge::development(CORE, TESTKIT),
        ObservedEdge::development(RUNTIME, TESTKIT),
        ObservedEdge::development(CAPI, TESTKIT),
    ];

    assert_eq!(validate(&graph_with(edges)), Vec::new());
}

#[test]
fn development_dependency_still_follows_the_allowlist() {
    let violations = validate(&graph_with(vec![ObservedEdge::development(
        CORE,
        PLATFORM_WINDOWS,
    )]));

    assert_eq!(forbidden_edges(&violations), vec![(CORE, PLATFORM_WINDOWS)]);
}

#[test]
fn dependency_on_the_maintenance_tool_is_rejected() {
    let edges = vec![
        ObservedEdge::production(RUNTIME, DEPENDENCY_CHECK),
        ObservedEdge::development(TESTKIT, DEPENDENCY_CHECK),
    ];

    let violations = validate(&graph_with(edges));

    assert_eq!(violations.len(), 2, "{violations:?}");
    assert!(
        violations
            .iter()
            .all(|violation| matches!(violation, Violation::MaintenanceToolDependency { .. }))
    );
}

#[test]
fn maintenance_tool_may_not_depend_on_a_product_package() {
    let violations = validate(&graph_with(vec![ObservedEdge::production(
        DEPENDENCY_CHECK,
        CORE,
    )]));

    assert_eq!(forbidden_edges(&violations), vec![(DEPENDENCY_CHECK, CORE)]);
}

#[test]
fn unrecognized_workspace_package_is_rejected_with_its_path() {
    let mut packages = baseline_packages();
    packages.push(ObservedPackage::new(
        "mado-pilot-experiment",
        "crates/automation/experiment",
    ));

    let violations = validate(&PackageGraph::new(packages, Vec::new()));

    assert_eq!(
        violations,
        vec![Violation::UnexpectedPackage {
            name: "mado-pilot-experiment".to_owned(),
            directory: "crates/automation/experiment".to_owned(),
        }]
    );
}

#[test]
fn missing_required_package_is_rejected_with_its_expected_path() {
    let packages = baseline_packages()
        .into_iter()
        .filter(|package| package.name != OCR)
        .collect::<Vec<_>>();

    let violations = validate(&PackageGraph::new(packages, Vec::new()));

    assert_eq!(
        violations,
        vec![Violation::MissingPackage {
            name: OCR.to_owned(),
            expected_directory: "crates/automation/ocr".to_owned(),
        }]
    );
}

#[test]
fn deferred_adapter_package_is_rejected() {
    let deferred = [
        ("mado-pilot-platform-adb", "crates/platform/adb"),
        ("mado-pilot-platform-browser", "crates/platform/browser"),
        (
            "mado-pilot-backend-apple-vision",
            "crates/backend/apple-vision",
        ),
    ];

    for (name, directory) in deferred {
        let mut packages = baseline_packages();
        packages.push(ObservedPackage::new(name, directory));

        let violations = validate(&PackageGraph::new(packages, Vec::new()));

        assert!(
            violations.iter().any(|violation| matches!(
                violation,
                Violation::DeferredPackage { name: found, .. } if found == name
            )),
            "{name} must be reported as deferred, got {violations:?}"
        );
        assert!(
            !violations
                .iter()
                .any(|violation| matches!(violation, Violation::UnexpectedPackage { .. })),
            "a deferred package must report the deferred reason, not a generic surprise"
        );
    }
}

#[test]
fn a_deferred_directory_reused_by_another_name_is_still_rejected() {
    let mut packages = baseline_packages();
    packages.push(ObservedPackage::new(
        "mado-pilot-android",
        "crates/platform/adb",
    ));

    let violations = validate(&PackageGraph::new(packages, Vec::new()));

    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::DeferredPackage { directory, .. } if directory == "crates/platform/adb"
        )),
        "{violations:?}"
    );
}

#[test]
fn required_package_outside_its_responsibility_group_is_rejected() {
    let packages = baseline_packages()
        .into_iter()
        .map(|package| {
            if package.name == BACKEND_OPENCV {
                ObservedPackage::new(BACKEND_OPENCV, "crates/automation/opencv")
            } else {
                package
            }
        })
        .collect::<Vec<_>>();

    let violations = validate(&PackageGraph::new(packages, Vec::new()));

    assert_eq!(
        violations,
        vec![Violation::MisplacedPackage {
            name: BACKEND_OPENCV.to_owned(),
            expected_directory: "crates/backend/opencv".to_owned(),
            actual_directory: "crates/automation/opencv".to_owned(),
        }]
    );
}

#[test]
fn graph_construction_normalizes_separators_and_ordering() {
    let windows_style = PackageGraph::new(
        vec![
            ObservedPackage::new(CAPTURE, "crates\\automation\\capture"),
            ObservedPackage::new(CORE, "./crates/automation/core"),
            ObservedPackage::new(CAPTURE, "crates/automation/capture"),
        ],
        vec![
            ObservedEdge::production(CAPTURE, CORE),
            ObservedEdge::production(CAPTURE, CORE),
        ],
    );
    let unix_style = PackageGraph::new(
        vec![
            ObservedPackage::new(CORE, "crates/automation/core"),
            ObservedPackage::new(CAPTURE, "crates/automation/capture"),
        ],
        vec![ObservedEdge::production(CAPTURE, CORE)],
    );

    assert_eq!(windows_style, unix_style);
    assert_eq!(windows_style.packages().len(), 2);
    assert_eq!(windows_style.edges().len(), 1);
}

#[test]
fn diagnostics_name_the_packages_and_the_allowed_destinations() {
    let violations = validate(&graph_with(vec![ObservedEdge::production(
        RUNTIME,
        BACKEND_ONNX,
    )]));

    let message = violations
        .first()
        .expect("the forbidden edge must be reported")
        .to_string();

    assert!(message.contains(RUNTIME), "{message}");
    assert!(message.contains(BACKEND_ONNX), "{message}");
    assert!(message.contains(ASSETS), "{message}");
}
