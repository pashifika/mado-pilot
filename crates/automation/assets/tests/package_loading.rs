//! What a valid package is, and what a caller gets when one loads.
//!
//! The property these tests exist to pin is equivalence: a directory, the same
//! files in memory, and an archive of the same files must commit the *same*
//! package. That is what lets a package be developed as a directory and shipped
//! as an archive without becoming a different package, and it is the one thing
//! a tracked valid fixture is for.

use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

use mado_pilot_assets::{
    AssetFaultKind, AssetLimits, ContentDigest, LoadStage, MANIFEST_PATH, MemoryPackage,
    PackageLoader, PackageSource,
};
#[cfg(unix)]
use mado_pilot_core::MonotonicInstant;
use mado_pilot_core::{CoordinateSpace, OperationContext, PixelExtent};
use mado_pilot_vision::TemplateEncoding;

mod support;

use support::{
    ArchiveEntry, PNG_SIGNATURE, TempDir, empty_manifest, load, tiny_archive, tiny_directory,
    tiny_manifest_bytes, tiny_memory_package, write_archive,
};
#[cfg(unix)]
use support::{ReplacingClock, WritingClock};

#[test]
fn the_tracked_tiny_package_loads_from_a_directory() {
    let package = load(&PackageSource::directory(tiny_directory())).expect("valid");

    assert_eq!(package.manifest().package_id(), "madopilot.fixture.tiny");
    assert_eq!(package.manifest().package_version(), "1.0.0");
    assert_eq!(package.manifest().license(), "Apache-2.0");
    assert_eq!(package.template_count(), 6);

    let provenance = package.manifest().provenance().expect("declared");
    assert_eq!(provenance.created_by(), "mado-pilot G-014 probe");
}

#[test]
fn a_directory_an_archive_and_memory_commit_the_same_package() {
    let from_directory = load(&PackageSource::directory(tiny_directory())).expect("valid");
    let from_archive = load(&PackageSource::archive_file(tiny_archive())).expect("valid");
    let from_memory = load(&PackageSource::memory(tiny_memory_package())).expect("valid");

    assert_eq!(from_directory, from_archive);
    assert_eq!(from_directory, from_memory);
}

#[test]
fn an_archive_read_from_bytes_matches_the_same_archive_read_from_a_file() {
    let bytes = std::fs::read(tiny_archive()).expect("readable");
    let from_file = load(&PackageSource::archive_file(tiny_archive())).expect("valid");
    let from_bytes = load(&PackageSource::archive_bytes(bytes)).expect("valid");

    assert_eq!(from_file, from_bytes);
}

#[cfg(unix)]
#[test]
fn an_archive_path_replaced_between_identity_checks_is_refused() {
    let package = TempDir::new("archive-identity-swap");
    let staged = TempDir::new("archive-identity-replacement");
    let bytes = std::fs::read(tiny_archive()).expect("readable fixture archive");
    let target = package.write("package.zip", &bytes);
    let replacement = staged.write("replacement.zip", &bytes);
    let clock = Arc::new(ReplacingClock::new(2, &target, replacement));
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(60)));

    let fault = PackageLoader::new()
        .load(&PackageSource::archive_file(&target), &context)
        .expect_err("an identity-changing replacement is refused");

    assert!(clock.replaced(), "the controlled replacement must have run");
    assert_eq!(fault.kind(), AssetFaultKind::SourceChanged);
    assert_eq!(fault.stage(), LoadStage::Source);
}

#[cfg(unix)]
#[test]
fn a_late_archive_path_replacement_invalidates_the_retained_snapshot() {
    let package = TempDir::new("archive-late-path-swap");
    let staged = TempDir::new("archive-late-path-replacement");
    let bytes = std::fs::read(tiny_archive()).expect("readable fixture archive");
    let target = package.write("package.zip", &bytes);
    let replacement = staged.write("replacement.zip", b"not an archive");
    let clock = Arc::new(ReplacingClock::new(3, &target, replacement));
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(60)));

    let fault = PackageLoader::new()
        .load(&PackageSource::archive_file(&target), &context)
        .expect_err("a removed source path invalidates the retained snapshot");

    assert!(clock.replaced(), "the controlled replacement must have run");
    assert_eq!(fault.kind(), AssetFaultKind::SourceChanged);
    assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
}

#[cfg(unix)]
#[test]
fn same_length_archive_mutation_invalidates_the_retained_snapshot() {
    let package = TempDir::new("archive-in-place-mutation");
    let bytes = std::fs::read(tiny_archive()).expect("readable fixture archive");
    let target = package.write("package.zip", &bytes);
    let mut replacement = bytes.into_boxed_slice();
    replacement[0] ^= 0xff;
    let clock = Arc::new(WritingClock::new(3, &target, replacement));
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(60)));

    let fault = PackageLoader::new()
        .load(&PackageSource::archive_file(&target), &context)
        .expect_err("in-place mutation invalidates the retained archive snapshot");

    assert!(clock.written(), "the controlled rewrite must have run");
    assert_eq!(fault.kind(), AssetFaultKind::SourceChanged);
    assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
}

#[test]
fn a_validated_template_resolves_into_a_vision_source_and_nothing_else() {
    let package = load(&PackageSource::directory(tiny_directory())).expect("valid");
    let template = package.resolve_template("template.0000").expect("declared");

    assert_eq!(template.id().as_str(), "template.0000");
    assert_eq!(template.extent(), PixelExtent::new(24, 24));
    assert_eq!(template.space(), CoordinateSpace::CapturePixels);
    assert_eq!(template.encoding(), TemplateEncoding::Png);
    assert_eq!(template.defaults().min_score(), 0.9);
    assert_eq!(template.defaults().max_results(), 8);
    assert!(
        template.content().starts_with(&[0x89, b'P', b'N', b'G']),
        "the resolved content is the image itself, not an asset wrapper"
    );
}

#[test]
fn resolving_the_same_template_twice_shares_its_content() {
    let package = load(&PackageSource::directory(tiny_directory())).expect("valid");
    let first = package.resolve_template("template.0001").expect("declared");
    let second = package.resolve_template("template.0001").expect("declared");

    assert!(std::ptr::eq(first.content(), second.content()));
}

#[test]
fn a_resolved_template_outlives_the_package_it_came_from() {
    let template = {
        let package = load(&PackageSource::directory(tiny_directory())).expect("valid");
        package.resolve_template("template.0002").expect("declared")
    };

    assert_eq!(template.extent(), PixelExtent::new(40, 40));
    assert!(!template.content().is_empty());
}

#[test]
fn an_unknown_template_identity_is_a_caller_error_rather_than_a_package_failure() {
    let package = load(&PackageSource::directory(tiny_directory())).expect("valid");
    let fault = package
        .resolve_template("template.9999")
        .expect_err("unknown");

    assert_eq!(fault.kind(), AssetFaultKind::UnknownTemplate);
    assert_eq!(fault.status(), mado_pilot_core::Status::InvalidArgument);
}

#[test]
fn template_identities_enumerate_in_their_own_order_not_the_manifests() {
    let package = load(&PackageSource::directory(tiny_directory())).expect("valid");
    let ids: Vec<&str> = package.template_ids().map(|id| id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();

    assert_eq!(ids, sorted);
    assert_eq!(ids.len(), 6);
}

#[test]
fn a_committed_package_is_unaffected_by_the_directory_it_came_from() {
    let temporary = TempDir::new("post-commit-mutation");
    temporary.fill_with_tiny_package();

    let package = load(&PackageSource::directory(temporary.path())).expect("valid");
    let before = package.resolve_template("template.0000").expect("declared");

    temporary.write("templates/0000-24x24.png", b"replaced");
    std::fs::remove_file(temporary.path().join(MANIFEST_PATH)).expect("removable");

    let after = package
        .resolve_template("template.0000")
        .expect("still declared");
    assert_eq!(before, after);
    assert_eq!(package.manifest().package_id(), "madopilot.fixture.tiny");
}

#[test]
fn a_memory_package_survives_the_buffers_it_was_built_from() {
    let manifest = tiny_manifest_bytes();
    let mut package_source = MemoryPackage::new().with_entry(MANIFEST_PATH, manifest);
    let root = tiny_directory();
    for index in 0..6u32 {
        let name = template_file_name(index);
        let mut bytes = std::fs::read(root.join("templates").join(&name)).expect("readable");
        package_source = package_source.with_entry(format!("templates/{name}"), bytes.clone());
        // Overwriting the caller's buffer must not reach the package.
        bytes.fill(0);
    }

    let package = load(&PackageSource::memory(package_source)).expect("valid");
    drop(root);

    let template = package.resolve_template("template.0004").expect("declared");
    assert!(template.content().starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn a_memory_package_can_be_assembled_with_nothing_but_the_public_surface() {
    // Every entry needs a declared digest and the loader verifies each one, so
    // before `ContentDigest::of` existed a caller assembling a package in
    // memory had to add a hashing crate to state a value this crate already
    // computed. Nothing outside `mado_pilot_assets` is used here.
    let content = PNG_SIGNATURE.to_vec();
    let digest = ContentDigest::of(&content);
    let manifest = format!(
        r#"{{
          "schema_version": 1,
          "package": {{ "id": "madopilot.test.assembled", "version": "1.0.0" }},
          "license": "Apache-2.0",
          "templates": [ {{
            "id": "assembled", "path": "templates/only.png", "width": 4, "height": 4,
            "coordinate_space": "capture_pixels",
            "content": {{ "algorithm": "sha256", "value": "{digest}" }},
            "match_defaults": {{ "min_score": 0.9, "max_results": 2 }}
          }} ]
        }}"#
    )
    .into_bytes();

    let package = load(&PackageSource::memory(
        MemoryPackage::new()
            .with_entry(MANIFEST_PATH, manifest)
            .with_entry("templates/only.png", content.clone()),
    ))
    .expect("a package whose digests were computed through the public constructor loads");

    assert_eq!(package.template_count(), 1);
    let template = package.resolve_template("assembled").expect("declared");
    assert_eq!(template.content(), content.as_slice());

    // The same computation the loader performs, which is why the load above
    // did not fail on its first hash.
    assert_eq!(ContentDigest::of(&content), digest);
    assert_eq!(ContentDigest::parse(&digest.to_string()), Some(digest));
}

#[test]
fn one_failing_entry_refuses_the_whole_package() {
    let temporary = TempDir::new("late-hash-failure");
    temporary.fill_with_tiny_package();
    // The last template in manifest order, so five entries have already been
    // read and verified when this one fails.
    temporary.write(
        "templates/0005-96x32.png",
        b"\x89PNG\r\n\x1a\nnot the declared bytes",
    );

    let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::HashMismatch);
    assert_eq!(fault.stage(), LoadStage::Expansion);
}

#[test]
fn a_refused_load_leaves_nothing_behind_for_the_next_one() {
    let temporary = TempDir::new("no-partial-trust");
    temporary.fill_with_tiny_package();
    temporary.write("templates/0003-48x48.png", b"\x89PNG\r\n\x1a\nwrong");

    assert!(load(&PackageSource::directory(temporary.path())).is_err());

    // Repairing the source is enough; nothing from the refused attempt is
    // remembered, and nothing from it was trusted.
    let original =
        std::fs::read(tiny_directory().join("templates/0003-48x48.png")).expect("readable");
    temporary.write("templates/0003-48x48.png", &original);

    let package = load(&PackageSource::directory(temporary.path())).expect("valid");
    assert_eq!(package.template_count(), 6);
}

#[test]
fn a_manifest_that_references_a_missing_entry_is_refused() {
    let temporary = TempDir::new("missing-entry");
    temporary.fill_with_tiny_package();
    std::fs::remove_file(temporary.path().join("templates/0002-40x40.png")).expect("removable");

    let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::MissingEntry);
    assert_eq!(fault.stage(), LoadStage::Expansion);
}

#[test]
fn a_package_with_no_manifest_is_refused_whatever_the_source() {
    let temporary = TempDir::new("no-manifest");
    temporary.write("templates/button.png", b"\x89PNG\r\n\x1a\n");

    let from_directory = load(&PackageSource::directory(temporary.path())).expect_err("refused");
    let from_memory = load(&PackageSource::memory(
        MemoryPackage::new().with_entry("templates/button.png", b"\x89PNG\r\n\x1a\n".to_vec()),
    ))
    .expect_err("refused");

    assert_eq!(from_directory.kind(), AssetFaultKind::MissingManifest);
    assert_eq!(from_directory.stage(), LoadStage::Manifest);
    assert_eq!(from_memory, from_directory);
}

#[test]
fn duplicate_normalized_names_are_refused_from_memory_too() {
    let package = MemoryPackage::new()
        .with_entry(MANIFEST_PATH, empty_manifest())
        .with_entry("templates/button.png", b"first".to_vec())
        .with_entry("./templates//button.png", b"second".to_vec());

    let fault = load(&PackageSource::memory(package)).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::DuplicatePath);
    assert_eq!(fault.stage(), LoadStage::EntryMetadata);
}

#[test]
fn distinct_template_ids_cannot_reference_the_same_normalized_entry() {
    let content = PNG_SIGNATURE.to_vec();
    let digest = ContentDigest::of(&content);
    let manifest = format!(
        r#"{{
          "schema_version": 1,
          "package": {{ "id": "madopilot.test.duplicate-reference", "version": "1.0.0" }},
          "license": "Apache-2.0",
          "templates": [
            {{
              "id": "first", "path": "templates/button.png", "width": 4, "height": 4,
              "coordinate_space": "capture_pixels",
              "content": {{ "algorithm": "sha256", "value": "{digest}" }},
              "match_defaults": {{ "min_score": 0.9, "max_results": 1 }}
            }},
            {{
              "id": "second", "path": "./templates//button.png", "width": 4, "height": 4,
              "coordinate_space": "capture_pixels",
              "content": {{ "algorithm": "sha256", "value": "{digest}" }},
              "match_defaults": {{ "min_score": 0.9, "max_results": 1 }}
            }}
          ]
        }}"#
    );
    let source = MemoryPackage::new()
        .with_entry(MANIFEST_PATH, manifest.into_bytes())
        .with_entry("templates/button.png", content);

    let fault = load(&PackageSource::memory(source)).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::DuplicatePath);
    assert_eq!(fault.stage(), LoadStage::Manifest);
}

#[test]
fn an_unsafe_name_is_refused_from_memory_too() {
    for name in ["/etc/passwd", "../outside.png", "C:/hosts", "a\\b.png"] {
        let package = MemoryPackage::new()
            .with_entry(MANIFEST_PATH, empty_manifest())
            .with_entry(name, b"x".to_vec());
        let fault = load(&PackageSource::memory(package)).expect_err("refused");

        assert_eq!(fault.kind(), AssetFaultKind::UnsafePath, "{name}");
        assert_eq!(fault.stage(), LoadStage::EntryMetadata, "{name}");
    }
}

#[test]
fn a_template_that_is_not_a_supported_image_is_refused_before_it_reaches_a_backend() {
    // The content hashes correctly and is the declared length; only its own
    // bytes say it is not an image this build accepts.
    let content = b"GIF89a not really a png".to_vec();
    let digest = hex_sha256(&content);
    let manifest = format!(
        r#"{{
          "schema_version": 1,
          "package": {{ "id": "p", "version": "1" }},
          "license": "Apache-2.0",
          "templates": [ {{
            "id": "t", "path": "t.gif", "width": 4, "height": 4,
            "coordinate_space": "capture_pixels",
            "content": {{ "algorithm": "sha256", "value": "{digest}" }},
            "match_defaults": {{ "min_score": 0.9, "max_results": 1 }}
          }} ]
        }}"#
    );
    let package = MemoryPackage::new()
        .with_entry(MANIFEST_PATH, manifest.into_bytes())
        .with_entry("t.gif", content);

    let fault = load(&PackageSource::memory(package)).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::UnsupportedContentEncoding);
    assert_eq!(fault.stage(), LoadStage::Expansion);
}

#[test]
fn an_unreferenced_entry_is_still_checked_but_never_expanded() {
    let temporary = TempDir::new("unreferenced-entry");
    temporary.fill_with_tiny_package();
    temporary.write("notes/README.txt", b"not referenced by the manifest");

    let package = load(&PackageSource::directory(temporary.path())).expect("valid");
    assert_eq!(package.template_count(), 6);

    // Its name is still subject to every rule, so an unsafe one refuses the
    // package even though nothing would have read it.
    let unsafe_names = MemoryPackage::new()
        .with_entry(MANIFEST_PATH, empty_manifest())
        .with_entry("../outside.txt", b"never read".to_vec());
    assert_eq!(
        load(&PackageSource::memory(unsafe_names))
            .expect_err("refused")
            .kind(),
        AssetFaultKind::UnsafePath
    );
}

#[test]
fn a_lowered_limit_refuses_a_package_the_ceiling_would_have_admitted() {
    let limits = AssetLimits::ceiling()
        .with_max_entry_count(2)
        .expect("below the ceiling");
    let loader = PackageLoader::with_limits(limits);

    let fault = loader
        .load(
            &PackageSource::directory(tiny_directory()),
            &OperationContext::new(),
        )
        .expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::ArchiveLimit);
    assert_eq!(fault.stage(), LoadStage::Source);
    assert!(
        load(&PackageSource::directory(tiny_directory())).is_ok(),
        "the same package still loads under the ceiling"
    );
}

#[test]
fn an_empty_archive_entry_is_content_a_manifest_may_simply_not_reference() {
    let archive = write_archive(
        &[
            ArchiveEntry::file(MANIFEST_PATH, &empty_manifest()),
            ArchiveEntry::file("filler/empty", b""),
        ],
        None,
    );
    let package = load(&PackageSource::archive_bytes(Arc::<[u8]>::from(archive))).expect("valid");

    assert_eq!(package.template_count(), 0);
    assert_eq!(package.manifest().package_id(), "madopilot.test.empty");
}

fn template_file_name(index: u32) -> String {
    let sizes = [
        "0000-24x24",
        "0001-32x32",
        "0002-40x40",
        "0003-48x48",
        "0004-64x64",
        "0005-96x32",
    ];
    format!(
        "{}.png",
        sizes[usize::try_from(index).expect("six templates")]
    )
}

fn hex_sha256(bytes: &[u8]) -> String {
    // The public constructor, so a caller assembling a manifest needs no
    // hashing dependency of its own. These tests used to carry three copies of
    // the same computation, which is what the gap looked like from the inside.
    ContentDigest::of(bytes).to_string()
}
