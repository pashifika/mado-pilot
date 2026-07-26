//! Deterministic tests for the shared-metadata and lint-opt-in rules.
//!
//! Every case builds synthetic member metadata, so results depend only on the
//! supplied values. Cargo is never invoked and no manifest is read.

use mado_pilot_dependency_check::graph::{
    CORE, FACADE, ObservedMetadata, REQUIRED_EDITION, REQUIRED_LICENSE, REQUIRED_PACKAGES, TESTKIT,
    Violation, manifest_inherits_workspace_lints, validate_metadata,
};

fn compliant_member(name: &str, directory: &str) -> ObservedMetadata {
    ObservedMetadata {
        name: name.to_owned(),
        directory: directory.to_owned(),
        version: "0.1.0".to_owned(),
        edition: REQUIRED_EDITION.to_owned(),
        rust_version: Some("1.97.1".to_owned()),
        license: Some(REQUIRED_LICENSE.to_owned()),
        repository: Some("https://github.com/pashifika/mado-pilot".to_owned()),
        publishable: false,
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

#[test]
fn a_workspace_with_consistent_metadata_is_accepted() {
    assert_eq!(validate_metadata(&compliant_members()), Vec::new());
}

#[test]
fn a_publishable_package_is_rejected() {
    let members = members_with(CORE, |member| member.publishable = true);

    assert_eq!(
        validate_metadata(&members),
        vec![Violation::PublishablePackage {
            name: CORE.to_owned()
        }]
    );
}

#[test]
fn an_overridden_license_is_rejected() {
    let members = members_with(CORE, |member| member.license = Some("MIT".to_owned()));

    assert_eq!(
        validate_metadata(&members),
        vec![Violation::UnexpectedLicense {
            name: CORE.to_owned(),
            license: Some("MIT".to_owned()),
            expected: REQUIRED_LICENSE.to_owned(),
        }]
    );
}

#[test]
fn a_missing_license_is_rejected() {
    let members = members_with(CORE, |member| member.license = None);

    assert_eq!(
        validate_metadata(&members),
        vec![Violation::UnexpectedLicense {
            name: CORE.to_owned(),
            license: None,
            expected: REQUIRED_LICENSE.to_owned(),
        }]
    );
}

#[test]
fn an_overridden_edition_is_rejected() {
    let members = members_with(CORE, |member| member.edition = "2021".to_owned());

    assert_eq!(
        validate_metadata(&members),
        vec![Violation::UnexpectedEdition {
            name: CORE.to_owned(),
            edition: "2021".to_owned(),
            expected: REQUIRED_EDITION.to_owned(),
        }]
    );
}

#[test]
fn a_member_that_does_not_inherit_workspace_lints_is_rejected() {
    let members = members_with(CORE, |member| member.inherits_workspace_lints = false);

    assert_eq!(
        validate_metadata(&members),
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
            validate_metadata(&members),
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
fn a_diverging_version_is_rejected_against_the_facade() {
    let members = members_with(TESTKIT, |member| member.version = "0.2.0".to_owned());

    assert_eq!(
        validate_metadata(&members),
        vec![Violation::InconsistentMetadata {
            name: TESTKIT.to_owned(),
            field: "version",
            value: Some("0.2.0".to_owned()),
            expected: Some("0.1.0".to_owned()),
            reference: FACADE.to_owned(),
        }]
    );
}

#[test]
fn a_diverging_rust_version_or_repository_is_rejected() {
    let members = members_with(CORE, |member| {
        member.rust_version = Some("1.90.0".to_owned());
        member.repository = None;
    });

    let violations = validate_metadata(&members);

    assert_eq!(violations.len(), 2, "{violations:?}");
    assert!(violations.iter().any(|violation| matches!(
        violation,
        Violation::InconsistentMetadata {
            field: "rust-version",
            ..
        }
    )));
    assert!(violations.iter().any(|violation| matches!(
        violation,
        Violation::InconsistentMetadata {
            field: "repository",
            ..
        }
    )));
}

#[test]
fn a_shared_version_bump_across_every_member_is_accepted() {
    // Agreement is the rule, not a hard-coded version, so a release bump does not
    // require a checker change.
    let members: Vec<ObservedMetadata> = compliant_members()
        .into_iter()
        .map(|mut member| {
            member.version = "0.4.2".to_owned();
            member.rust_version = Some("1.99.0".to_owned());
            member
        })
        .collect();

    assert_eq!(validate_metadata(&members), Vec::new());
}

#[test]
fn metadata_agreement_is_skipped_when_the_facade_is_absent() {
    // A missing facade is already an inventory violation; reporting every other
    // member as inconsistent on top of that would bury the real finding.
    let members: Vec<ObservedMetadata> = compliant_members()
        .into_iter()
        .filter(|member| member.name != FACADE)
        .map(|mut member| {
            member.version = "9.9.9".to_owned();
            member
        })
        .collect();

    assert_eq!(validate_metadata(&members), Vec::new());
}

#[test]
fn the_canonical_lints_stanza_is_recognized() {
    let manifest = "\
[package]
name = \"mado-pilot-core\"

[lints]
workspace = true
";

    assert!(manifest_inherits_workspace_lints(manifest));
}

#[test]
fn the_dotted_lints_form_is_recognized() {
    assert!(manifest_inherits_workspace_lints(
        "[package]\nname = \"x\"\nlints.workspace = true\n"
    ));
}

#[test]
fn extra_whitespace_and_comments_do_not_hide_the_lints_stanza() {
    let manifest = "\
[package]
name = \"x\"

# Inherit the workspace policy.
[lints]
   workspace   =   true   # explicit opt-in
";

    assert!(manifest_inherits_workspace_lints(manifest));
}

#[test]
fn a_manifest_without_a_lints_stanza_is_reported_as_missing() {
    assert!(!manifest_inherits_workspace_lints(
        "[package]\nname = \"x\"\npublish = false\n"
    ));
}

#[test]
fn workspace_false_is_not_an_opt_in() {
    assert!(!manifest_inherits_workspace_lints(
        "[lints]\nworkspace = false\n"
    ));
}

#[test]
fn a_workspace_key_in_another_table_is_not_an_opt_in() {
    // `[dependencies] serde = { workspace = true }` must not be mistaken for the
    // lint opt-in, and neither must a `workspace = true` in any other table.
    let manifest = "\
[package]
name = \"x\"

[dependencies]
workspace = true

[features]
default = []
";

    assert!(!manifest_inherits_workspace_lints(manifest));
}

#[test]
fn a_commented_out_lints_stanza_is_not_an_opt_in() {
    assert!(!manifest_inherits_workspace_lints(
        "[package]\nname = \"x\"\n# [lints]\n# workspace = true\n"
    ));
}
