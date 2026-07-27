//! Deterministic tests for the manifest-text reader.
//!
//! Every case supplies manifest text directly, so results depend only on that
//! text. Cargo is never invoked and no file is read.

use mado_pilot_dependency_check::manifest::{
    Manifest, PACKAGE_TABLE, TOOLCHAIN_TABLE, WORKSPACE_PACKAGE_TABLE,
};

fn inherits_lints(manifest: &str) -> bool {
    Manifest::parse(manifest).inherits_workspace_lints()
}

fn inherited(manifest: &str) -> Vec<String> {
    Manifest::parse(manifest)
        .inherited_package_fields()
        .into_iter()
        .collect()
}

#[test]
fn the_canonical_lints_stanza_is_recognized() {
    let manifest = "\
[package]
name = \"mado-pilot-core\"

[lints]
workspace = true
";

    assert!(inherits_lints(manifest));
}

#[test]
fn the_dotted_lints_form_is_recognized_at_document_scope() {
    // At document scope the dotted key belongs to the root `lints` table, which is
    // what Cargo reads for workspace lint inheritance.
    assert!(inherits_lints(
        "lints.workspace = true\n[package]\nname = \"x\"\n"
    ));
}

#[test]
fn the_dotted_lints_form_inside_the_package_table_is_not_an_opt_in() {
    // This text sets `package.lints.workspace`, which Cargo ignores: the workspace
    // Rust, rustdoc, and Clippy lints stay disabled for the package. Accepting it
    // would let the checker certify a package whose lints are off.
    assert!(!inherits_lints(
        "[package]\nname = \"x\"\nlints.workspace = true\n"
    ));
}

#[test]
fn the_dotted_lints_form_inside_another_table_is_not_an_opt_in() {
    assert!(!inherits_lints(
        "[lints]\nworkspace = false\n\n[dependencies]\nlints.workspace = true\n"
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

    assert!(inherits_lints(manifest));
}

#[test]
fn a_manifest_without_a_lints_stanza_is_reported_as_missing() {
    assert!(!inherits_lints(
        "[package]\nname = \"x\"\npublish = false\n"
    ));
}

#[test]
fn workspace_false_is_not_an_opt_in() {
    assert!(!inherits_lints("[lints]\nworkspace = false\n"));
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
serde = { workspace = true }

[features]
default = []
";

    assert!(!inherits_lints(manifest));
}

#[test]
fn a_commented_out_lints_stanza_is_not_an_opt_in() {
    assert!(!inherits_lints(
        "[package]\nname = \"x\"\n# [lints]\n# workspace = true\n"
    ));
}

#[test]
fn a_lints_stanza_after_another_table_is_still_recognized() {
    // Table state must follow the last header, not the first one.
    let manifest = "\
[package]
name = \"x\"

[dependencies]
serde = { workspace = true }

[lints]
workspace = true
";

    assert!(inherits_lints(manifest));
}

#[test]
fn only_the_exact_lints_table_is_an_opt_in() {
    // These headers carry the whole weight of the rule: `[lints]` is the only table
    // Cargo reads for workspace lint inheritance. A prefix or suffix match — such as
    // a later refactor to `table.starts_with("lints")` — would silently accept a
    // package whose lints are disabled, so each near miss is locked in here.
    let rejected = [
        "[lints.rust]\nworkspace = true\n",
        "[lints.clippy]\nworkspace = true\n",
        "[lints.workspace]\nworkspace = true\n",
        "[workspace.lints]\nworkspace = true\n",
        "[workspace.lints.rust]\nworkspace = true\n",
        "[target.'cfg(unix)'.lints]\nworkspace = true\n",
        "[lintsx]\nworkspace = true\n",
        "[[lints]]\nworkspace = true\n",
    ];

    for manifest in rejected {
        assert!(
            !inherits_lints(manifest),
            "{manifest:?} must not count as the lint opt-in"
        );
    }
}

#[test]
fn layout_variations_of_the_lints_stanza_are_still_an_opt_in() {
    // The rule is about the table, not about spelling, so a header with inner
    // spaces, CRLF line endings, and tabs must all still be recognized.
    let accepted = [
        "[ lints ]\nworkspace = true\n",
        "[package]\r\nname = \"x\"\r\n\r\n[lints]\r\nworkspace = true\r\n",
        "[lints]\n\tworkspace\t=\ttrue\n",
        "[lints] # inherit the workspace policy\nworkspace = true\n",
        "lints = { workspace = true }\n",
    ];

    for manifest in accepted {
        assert!(
            inherits_lints(manifest),
            "{manifest:?} must count as the lint opt-in"
        );
    }
}

#[test]
fn an_array_of_tables_header_does_not_leak_into_a_package_field() {
    // `[[bin]]` opens a different table, so a key under it is neither a `[package]`
    // field nor a continuation of the table before it.
    let manifest = "\
[package]
name = \"x\"
edition.workspace = true

[[bin]]
name = \"x\"
version.workspace = true
";

    assert_eq!(inherited(manifest), vec!["edition".to_owned()]);
}

#[test]
fn a_byte_order_mark_does_not_hide_the_first_table() {
    // A UTF-8 BOM is not whitespace, so without stripping it the first header goes
    // unrecognized and every visible declaration under it is reported as missing.
    let manifest = "\u{feff}[package]\nname = \"x\"\nversion.workspace = true\n";

    assert_eq!(inherited(manifest), vec!["version".to_owned()]);
    assert!(inherits_lints("\u{feff}[lints]\nworkspace = true\n"));
}

#[test]
fn a_multi_line_string_cannot_forge_an_inheritance_declaration() {
    // A multi-line string's contents are a value, never assignments. Reading them as
    // assignments would report a field as inherited when the manifest only mentions
    // the text inside a description.
    let basic = "\
[package]
name = \"x\"
description = \"\"\"
version.workspace = true
\"\"\"
edition.workspace = true
";
    let literal = "\
[package]
name = \"x\"
description = '''
version.workspace = true
'''
edition.workspace = true
";

    assert_eq!(inherited(basic), vec!["edition".to_owned()]);
    assert_eq!(inherited(literal), vec!["edition".to_owned()]);
}

#[test]
fn a_multi_line_string_cannot_forge_the_lints_stanza() {
    let manifest = "\
[package]
name = \"x\"
description = \"\"\"
[lints]
workspace = true
\"\"\"
";

    assert!(!inherits_lints(manifest));
}

#[test]
fn a_value_after_a_multi_line_string_is_still_read() {
    // Closing the string must resume parsing rather than swallow the rest of the
    // file, and the unreadable multi-line value itself reads as absent.
    let manifest = "\
[workspace.package]
description = \"\"\"
a
\"\"\"
version = \"0.1.0\"
";

    let manifest = Manifest::parse(manifest);

    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "version"),
        Some("0.1.0")
    );
    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "description"),
        None
    );
}

#[test]
fn a_multi_line_string_spliced_into_an_identifier_does_not_forge_a_key() {
    // Discarding the string must leave something behind, or `work"""x"""space`
    // collapses into `workspace`. A single space would not be enough, because key
    // spelling ignores whitespace: `work space` compacts to `workspace` too.
    for delimiter in ["\"\"\"", "'''"] {
        let lints = format!("[lints]\nwork{delimiter}x{delimiter}space = true\n");
        let field = format!("[package]\nversion.work{delimiter}x{delimiter}space = true\n");

        assert!(
            !inherits_lints(&lints),
            "{lints:?} must not count as the lint opt-in"
        );
        assert_eq!(
            inherited(&field),
            Vec::<String>::new(),
            "{field:?} must not report an inherited field"
        );
    }
}

#[test]
fn a_single_line_multi_line_string_is_read_as_absent() {
    let manifest = Manifest::parse("[workspace.package]\nversion = \"\"\"0.1.0\"\"\"\n");

    assert_eq!(manifest.string(WORKSPACE_PACKAGE_TABLE, "version"), None);
}

#[test]
fn dotted_package_fields_are_reported_as_inherited() {
    let manifest = "\
[package]
name = \"x\"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false
";

    assert_eq!(
        inherited(manifest),
        vec![
            "edition".to_owned(),
            "license".to_owned(),
            "repository".to_owned(),
            "rust-version".to_owned(),
            "version".to_owned(),
        ]
    );
}

#[test]
fn the_inline_table_form_is_reported_as_inherited() {
    // Cargo accepts `version = { workspace = true }` as the same declaration.
    assert_eq!(
        inherited("[package]\nname = \"x\"\nversion = { workspace = true }\n"),
        vec!["version".to_owned()]
    );
}

#[test]
fn a_hard_coded_field_is_not_reported_as_inherited() {
    let manifest = "\
[package]
name = \"x\"
version = \"0.1.0\"
edition.workspace = true
";

    assert_eq!(inherited(manifest), vec!["edition".to_owned()]);
}

#[test]
fn an_inheritance_declaration_in_another_table_is_not_a_package_field() {
    let manifest = "\
[dependencies]
serde = { workspace = true }

[dev-dependencies]
version.workspace = true
";

    assert_eq!(inherited(manifest), Vec::<String>::new());
}

#[test]
fn a_workspace_false_field_is_not_reported_as_inherited() {
    assert_eq!(
        inherited("[package]\nname = \"x\"\nversion.workspace = false\n"),
        Vec::<String>::new()
    );
}

#[test]
fn shared_workspace_values_are_read_from_the_workspace_package_table() {
    let manifest = "\
[workspace]
members = [
    \"crates/mado-pilot\",
    \"tools/dependency-check\",
]

[workspace.package]
version = \"0.1.0\"
edition = \"2024\"
rust-version = \"1.97.1\"
license = \"Apache-2.0\"
repository = \"https://github.com/pashifika/mado-pilot\"

[workspace.lints.rust]
missing_docs = \"warn\"
";

    let manifest = Manifest::parse(manifest);

    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "version"),
        Some("0.1.0")
    );
    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "rust-version"),
        Some("1.97.1")
    );
    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "repository"),
        Some("https://github.com/pashifika/mado-pilot")
    );
    // A member-table key of the same name must not leak into the workspace view.
    assert_eq!(manifest.string(PACKAGE_TABLE, "version"), None);
}

#[test]
fn an_empty_shared_value_reads_as_absent() {
    // A blank field must not satisfy a presence requirement.
    let manifest = Manifest::parse("[workspace.package]\nrepository = \"\"\n");

    assert_eq!(manifest.string(WORKSPACE_PACKAGE_TABLE, "repository"), None);
    assert_eq!(
        Manifest::parse("[workspace.package]\nrepository = ''\n")
            .string(WORKSPACE_PACKAGE_TABLE, "repository"),
        None
    );
}

#[test]
fn a_literal_string_is_read_as_a_value() {
    // `channel = '1.98.0'` is valid TOML. Reading it as absent would report
    // "pins channel `no channel`" for a file that plainly declares one.
    assert_eq!(
        Manifest::parse("[toolchain]\nchannel = '1.98.0'\n").string(TOOLCHAIN_TABLE, "channel"),
        Some("1.98.0")
    );
    assert_eq!(
        Manifest::parse("[workspace.package]\nrepository = 'https://host/repo'\n")
            .string(WORKSPACE_PACKAGE_TABLE, "repository"),
        Some("https://host/repo")
    );
}

#[test]
fn a_value_with_mismatched_delimiters_reads_as_absent() {
    let manifest = Manifest::parse("[workspace.package]\nversion = \"0.1.0'\n");

    assert_eq!(manifest.string(WORKSPACE_PACKAGE_TABLE, "version"), None);
}

#[test]
fn a_comment_marker_inside_a_string_is_kept() {
    let manifest = Manifest::parse("[workspace.package]\nrepository = \"https://host/a#b\"\n");

    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "repository"),
        Some("https://host/a#b")
    );
    assert_eq!(
        Manifest::parse("[workspace.package]\nrepository = 'https://host/a#b' # note\n")
            .string(WORKSPACE_PACKAGE_TABLE, "repository"),
        Some("https://host/a#b")
    );
}

#[test]
fn an_apostrophe_inside_a_basic_string_does_not_start_a_literal_string() {
    // Tracking `'` as a delimiter must not fire inside a basic string, or the rest of
    // the line — including a real comment marker — would be misread.
    let manifest = Manifest::parse(
        "[workspace.package]\ndescription = \"Bob's tool\"\nversion = \"0.1.0\" # note\n",
    );

    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "description"),
        Some("Bob's tool")
    );
    assert_eq!(
        manifest.string(WORKSPACE_PACKAGE_TABLE, "version"),
        Some("0.1.0")
    );
}

#[test]
fn the_toolchain_channel_is_read_from_the_pin_file() {
    let pin = "\
# The pinned toolchain is the tested minimum supported Rust version.
[toolchain]
channel = \"1.97.1\"
components = [\"rustfmt\", \"clippy\"]
";

    assert_eq!(
        Manifest::parse(pin).string(TOOLCHAIN_TABLE, "channel"),
        Some("1.97.1")
    );
}
