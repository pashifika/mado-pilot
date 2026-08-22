//! Deterministic tests for the workspace metadata contract and lint opt-in.
//!
//! Every case builds synthetic workspace and member metadata, so results depend
//! only on the supplied values. Cargo is never invoked and no manifest is read.

use std::collections::BTreeSet;

use mado_pilot_dependency_check::graph::{
    CORE, INHERITED_PACKAGE_FIELDS, ObservedMetadata, ObservedWorkspaceMetadata, REQUIRED_EDITION,
    REQUIRED_LICENSE, REQUIRED_PACKAGES, REQUIRED_REPOSITORY, REQUIRED_RUST_VERSION,
    REQUIRED_VERSION, TESTKIT, Violation, validate_metadata,
};

fn compliant_workspace() -> ObservedWorkspaceMetadata {
    ObservedWorkspaceMetadata {
        version: Some(REQUIRED_VERSION.to_owned()),
        edition: Some(REQUIRED_EDITION.to_owned()),
        rust_version: Some(REQUIRED_RUST_VERSION.to_owned()),
        license: Some(REQUIRED_LICENSE.to_owned()),
        repository: Some(REQUIRED_REPOSITORY.to_owned()),
        toolchain_channel: Some(REQUIRED_RUST_VERSION.to_owned()),
    }
}

fn compliant_member(name: &str, directory: &str) -> ObservedMetadata {
    ObservedMetadata {
        name: name.to_owned(),
        directory: directory.to_owned(),
        version: REQUIRED_VERSION.to_owned(),
        edition: REQUIRED_EDITION.to_owned(),
        rust_version: Some(REQUIRED_RUST_VERSION.to_owned()),
        license: Some(REQUIRED_LICENSE.to_owned()),
        repository: Some(REQUIRED_REPOSITORY.to_owned()),
        publishable: false,
        inherited_fields: INHERITED_PACKAGE_FIELDS
            .iter()
            .map(|field| (*field).to_owned())
            .collect(),
        inherits_workspace_lints: true,
    }
}

fn compliant_members() -> Vec<ObservedMetadata> {
    REQUIRED_PACKAGES
        .iter()
        .map(|package| compliant_member(package.name, package.directory))
        .collect()
}

fn members_with(name: &str, mutate: impl FnOnce(&mut ObservedMetadata)) -> Vec<ObservedMetadata> {
    let mut members = compliant_members();
    let member = members
        .iter_mut()
        .find(|member| member.name == name)
        .expect("the test names a required package");
    mutate(member);
    members
}

fn every_member_with(mutate: impl Fn(&mut ObservedMetadata)) -> Vec<ObservedMetadata> {
    let mut members = compliant_members();
    for member in &mut members {
        mutate(member);
    }
    members
}

fn violations_of(members: &[ObservedMetadata]) -> Vec<Violation> {
    validate_metadata(&compliant_workspace(), members)
}

#[test]
fn a_workspace_with_consistent_metadata_is_accepted() {
    assert_eq!(violations_of(&compliant_members()), Vec::new());
}

#[test]
fn a_publishable_package_is_rejected() {
    let members = members_with(CORE, |member| member.publishable = true);

    assert_eq!(
        violations_of(&members),
        vec![Violation::PublishablePackage {
            name: CORE.to_owned()
        }]
    );
}

#[test]
fn an_overridden_license_is_rejected() {
    let members = members_with(CORE, |member| member.license = Some("MIT".to_owned()));

    assert_eq!(
        violations_of(&members),
        vec![Violation::InconsistentMetadata {
            name: CORE.to_owned(),
            field: "license",
            value: Some("MIT".to_owned()),
            expected: Some(REQUIRED_LICENSE.to_owned()),
        }]
    );
}

#[test]
fn a_missing_license_is_rejected() {
    let members = members_with(CORE, |member| member.license = None);

    assert_eq!(
        violations_of(&members),
        vec![Violation::InconsistentMetadata {
            name: CORE.to_owned(),
            field: "license",
            value: None,
            expected: Some(REQUIRED_LICENSE.to_owned()),
        }]
    );
}

#[test]
fn an_overridden_edition_is_rejected() {
    let members = members_with(CORE, |member| member.edition = "2021".to_owned());

    assert_eq!(
        violations_of(&members),
        vec![Violation::InconsistentMetadata {
            name: CORE.to_owned(),
            field: "edition",
            value: Some("2021".to_owned()),
            expected: Some(REQUIRED_EDITION.to_owned()),
        }]
    );
}

#[test]
fn a_member_that_does_not_inherit_workspace_lints_is_rejected() {
    let members = members_with(CORE, |member| member.inherits_workspace_lints = false);

    assert_eq!(
        violations_of(&members),
        vec![Violation::MissingWorkspaceLints {
            name: CORE.to_owned(),
            directory: "crates/automation/core".to_owned(),
        }]
    );
}

#[test]
fn every_required_member_must_inherit_workspace_lints() {
    // The escape this rule closes applies to any member, not just one.
    for required in REQUIRED_PACKAGES {
        let members = members_with(required.name, |member| {
            member.inherits_workspace_lints = false;
        });
        assert_eq!(
            violations_of(&members),
            vec![Violation::MissingWorkspaceLints {
                name: required.name.to_owned(),
                directory: required.directory.to_owned(),
            }],
            "{} must be required to inherit the lint policy",
            required.name
        );
    }
}

#[test]
fn a_diverging_version_is_rejected_against_the_root_workspace() {
    let members = members_with(TESTKIT, |member| member.version = "0.3.0".to_owned());

    assert_eq!(
        violations_of(&members),
        vec![Violation::InconsistentMetadata {
            name: TESTKIT.to_owned(),
            field: "version",
            value: Some("0.3.0".to_owned()),
            expected: Some(REQUIRED_VERSION.to_owned()),
        }]
    );
}

#[test]
fn a_diverging_rust_version_or_repository_is_rejected() {
    let members = members_with(CORE, |member| {
        member.rust_version = Some("1.90.0".to_owned());
        member.repository = None;
    });

    assert_eq!(
        violations_of(&members),
        vec![
            Violation::InconsistentMetadata {
                name: CORE.to_owned(),
                field: "rust-version",
                value: Some("1.90.0".to_owned()),
                expected: Some(REQUIRED_RUST_VERSION.to_owned()),
            },
            Violation::InconsistentMetadata {
                name: CORE.to_owned(),
                field: "repository",
                value: None,
                expected: Some(REQUIRED_REPOSITORY.to_owned()),
            },
        ]
    );
}

#[test]
fn the_facade_is_not_exempt_from_the_shared_contract() {
    // The facade used to be the reference value, which made its own drift invisible.
    let members = members_with("mado-pilot", |member| member.version = "9.9.9".to_owned());

    assert_eq!(
        violations_of(&members),
        vec![Violation::InconsistentMetadata {
            name: "mado-pilot".to_owned(),
            field: "version",
            value: Some("9.9.9".to_owned()),
            expected: Some(REQUIRED_VERSION.to_owned()),
        }]
    );
}

#[test]
fn a_version_and_rust_version_bump_across_the_whole_workspace_is_rejected() {
    // Agreement between members is not enough: the root declaration and the
    // toolchain pin are anchored to the repository contract, so a release bump
    // has to be an intentional, reviewed edit rather than a silent drift.
    let workspace = ObservedWorkspaceMetadata {
        version: Some("0.4.2".to_owned()),
        rust_version: Some("1.99.0".to_owned()),
        toolchain_channel: Some("1.99.0".to_owned()),
        ..compliant_workspace()
    };
    let members = every_member_with(|member| {
        member.version = "0.4.2".to_owned();
        member.rust_version = Some("1.99.0".to_owned());
    });

    assert_eq!(
        validate_metadata(&workspace, &members),
        vec![
            Violation::UnexpectedWorkspaceMetadata {
                field: "version",
                value: Some("0.4.2".to_owned()),
                expected: REQUIRED_VERSION,
            },
            Violation::UnexpectedWorkspaceMetadata {
                field: "rust-version",
                value: Some("1.99.0".to_owned()),
                expected: REQUIRED_RUST_VERSION,
            },
            Violation::UnexpectedToolchainChannel {
                channel: Some("1.99.0".to_owned()),
                expected: REQUIRED_RUST_VERSION,
            },
        ]
    );
}

#[test]
fn a_rust_version_missing_from_the_whole_workspace_is_rejected() {
    // Every value agreeing as unset used to pass, which dropped the tested minimum
    // supported Rust version without any check failing.
    let workspace = ObservedWorkspaceMetadata {
        rust_version: None,
        ..compliant_workspace()
    };
    let members = every_member_with(|member| {
        member.rust_version = None;
        member.inherited_fields.remove("rust-version");
    });

    let violations = validate_metadata(&workspace, &members);

    assert!(
        violations.contains(&Violation::UnexpectedWorkspaceMetadata {
            field: "rust-version",
            value: None,
            expected: REQUIRED_RUST_VERSION,
        }),
        "{violations:?}"
    );
}

#[test]
fn a_repository_missing_from_the_whole_workspace_is_rejected() {
    let workspace = ObservedWorkspaceMetadata {
        repository: None,
        ..compliant_workspace()
    };
    let members = every_member_with(|member| {
        member.repository = None;
        member.inherited_fields.remove("repository");
    });

    let violations = validate_metadata(&workspace, &members);

    assert!(
        violations.contains(&Violation::UnexpectedWorkspaceMetadata {
            field: "repository",
            value: None,
            expected: REQUIRED_REPOSITORY,
        }),
        "{violations:?}"
    );
}

#[test]
fn every_shared_root_value_is_anchored_to_the_phase_0_contract() {
    let cases: [(&'static str, &str, &'static str, ObservedWorkspaceMetadata); 3] = [
        (
            "edition",
            "2021",
            REQUIRED_EDITION,
            ObservedWorkspaceMetadata {
                edition: Some("2021".to_owned()),
                ..compliant_workspace()
            },
        ),
        (
            "license",
            "MIT",
            REQUIRED_LICENSE,
            ObservedWorkspaceMetadata {
                license: Some("MIT".to_owned()),
                ..compliant_workspace()
            },
        ),
        (
            "repository",
            "https://example.invalid/fork",
            REQUIRED_REPOSITORY,
            ObservedWorkspaceMetadata {
                repository: Some("https://example.invalid/fork".to_owned()),
                ..compliant_workspace()
            },
        ),
    ];

    for (field, value, expected, workspace) in cases {
        assert_eq!(
            validate_metadata(&workspace, &[]),
            vec![Violation::UnexpectedWorkspaceMetadata {
                field,
                value: Some(value.to_owned()),
                expected,
            }],
            "the root `{field}` must be anchored to the contract"
        );
    }
}

#[test]
fn a_toolchain_pin_that_disagrees_with_the_declared_rust_version_is_rejected() {
    // Both the pin and the root declaration are anchored to the same contract value,
    // so changing either one alone is reported.
    let pin_moved = ObservedWorkspaceMetadata {
        toolchain_channel: Some("1.98.0".to_owned()),
        ..compliant_workspace()
    };
    assert_eq!(
        validate_metadata(&pin_moved, &[]),
        vec![Violation::UnexpectedToolchainChannel {
            channel: Some("1.98.0".to_owned()),
            expected: REQUIRED_RUST_VERSION,
        }]
    );

    let manifest_moved = ObservedWorkspaceMetadata {
        rust_version: Some("1.98.0".to_owned()),
        ..compliant_workspace()
    };
    assert_eq!(
        validate_metadata(&manifest_moved, &[]),
        vec![Violation::UnexpectedWorkspaceMetadata {
            field: "rust-version",
            value: Some("1.98.0".to_owned()),
            expected: REQUIRED_RUST_VERSION,
        }]
    );
}

#[test]
fn a_missing_toolchain_pin_is_rejected() {
    let workspace = ObservedWorkspaceMetadata {
        toolchain_channel: None,
        ..compliant_workspace()
    };

    assert_eq!(
        validate_metadata(&workspace, &[]),
        vec![Violation::UnexpectedToolchainChannel {
            channel: None,
            expected: REQUIRED_RUST_VERSION,
        }]
    );
}

#[test]
fn a_hard_coded_shared_value_is_rejected_even_when_it_agrees() {
    // Cargo metadata reports resolved values, so a member that repeats the
    // contract value instead of inheriting it looks identical there. Only the
    // manifest declaration distinguishes the two, and an uninherited field drifts
    // the next time the workspace value changes.
    let members = members_with(CORE, |member| {
        member.inherited_fields.remove("version");
    });

    assert_eq!(
        violations_of(&members),
        vec![Violation::MissingWorkspaceInheritance {
            name: CORE.to_owned(),
            directory: "crates/automation/core".to_owned(),
            field: "version",
        }]
    );
}

#[test]
fn every_shared_field_must_be_inherited_by_every_member() {
    for field in INHERITED_PACKAGE_FIELDS {
        for required in REQUIRED_PACKAGES {
            let members = members_with(required.name, |member| {
                member.inherited_fields.remove(*field);
            });
            assert_eq!(
                violations_of(&members),
                vec![Violation::MissingWorkspaceInheritance {
                    name: required.name.to_owned(),
                    directory: required.directory.to_owned(),
                    field,
                }],
                "{} must inherit `{field}`",
                required.name
            );
        }
    }
}

#[test]
fn a_member_that_inherits_nothing_reports_every_shared_field() {
    let members = members_with(CORE, |member| {
        member.inherited_fields = BTreeSet::new();
    });

    let reported: Vec<&str> = violations_of(&members)
        .iter()
        .filter_map(|violation| match violation {
            Violation::MissingWorkspaceInheritance { field, .. } => Some(*field),
            _ => None,
        })
        .collect();

    assert_eq!(reported, INHERITED_PACKAGE_FIELDS.to_vec());
}
