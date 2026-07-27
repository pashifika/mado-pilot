//! The `G-014` conformance suite.
//!
//! Every tracked adversarial fixture crosses exactly one rule and stays inside
//! every other one, so the outcome identifies which check stopped it. Each case
//! asserts the failure category **and** the stage: a fixture rejected later than
//! its documented stage means an earlier guard is missing, and fails here even
//! though the package was refused.
//!
//! The table below is transcribed from `fixtures/assets/g-014/README.md`. It is
//! deliberately a literal copy rather than something derived, so a change to
//! either document has to be made in both places and cannot pass unnoticed.

use std::path::Path;

use mado_pilot_assets::{AssetFaultKind, AssetLimits, LoadStage, PackageLoader, PackageSource};
use mado_pilot_core::{OperationContext, Status};

mod support;

use support::{ArchiveEntry, adversarial, empty_manifest, fixture_root, load, write_archive};

/// The expected outcome of every tracked adversarial fixture.
const ADVERSARIAL: &[(&str, AssetFaultKind, LoadStage)] = &[
    (
        "path-absolute-posix.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-absolute-drive.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-unc-root.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-traversal.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-traversal-inner.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-backslash-separator.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-embedded-nul.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-non-utf8.zip",
        AssetFaultKind::UnsafePath,
        LoadStage::EntryMetadata,
    ),
    (
        "path-duplicate-normalized.zip",
        AssetFaultKind::DuplicatePath,
        LoadStage::EntryMetadata,
    ),
    (
        "entry-symlink.zip",
        AssetFaultKind::UnsupportedEntryType,
        LoadStage::EntryMetadata,
    ),
    (
        "entry-fifo.zip",
        AssetFaultKind::UnsupportedEntryType,
        LoadStage::EntryMetadata,
    ),
    (
        "entry-character-device.zip",
        AssetFaultKind::UnsupportedEntryType,
        LoadStage::EntryMetadata,
    ),
    (
        "entry-directory-name-collision.zip",
        AssetFaultKind::UnsupportedEntryType,
        LoadStage::EntryMetadata,
    ),
    (
        "bomb-entry-count-declared.zip",
        AssetFaultKind::ArchiveLimit,
        LoadStage::DirectoryPreParse,
    ),
    (
        "bomb-total-uncompressed-declared.zip",
        AssetFaultKind::ArchiveLimit,
        LoadStage::DirectoryOpen,
    ),
    (
        "bomb-entry-uncompressed-declared.zip",
        AssetFaultKind::ArchiveLimit,
        LoadStage::EntryMetadata,
    ),
    (
        "bomb-compression-ratio.zip",
        AssetFaultKind::ArchiveLimit,
        LoadStage::EntryMetadata,
    ),
    (
        "manifest-oversize.zip",
        AssetFaultKind::ArchiveLimit,
        LoadStage::EntryMetadata,
    ),
    (
        "bomb-understated-declaration.zip",
        AssetFaultKind::DeclaredSizeMismatch,
        LoadStage::Expansion,
    ),
    (
        "manifest-missing.zip",
        AssetFaultKind::MissingManifest,
        LoadStage::Manifest,
    ),
    (
        "manifest-malformed.zip",
        AssetFaultKind::MalformedManifest,
        LoadStage::Manifest,
    ),
    (
        "manifest-unsupported-schema.zip",
        AssetFaultKind::UnsupportedSchemaVersion,
        LoadStage::Manifest,
    ),
    (
        "hash-mismatch.zip",
        AssetFaultKind::HashMismatch,
        LoadStage::Expansion,
    ),
];

#[test]
fn every_adversarial_fixture_is_refused_by_its_documented_guard() {
    for &(name, expected_kind, expected_stage) in ADVERSARIAL {
        let fault = load(&PackageSource::archive_file(adversarial(name)))
            .expect_err("an adversarial fixture must never commit a package");

        assert_eq!(
            (fault.kind(), fault.stage()),
            (expected_kind, expected_stage),
            "{name} was refused as {} at {}, not as {expected_kind} at {expected_stage}",
            fault.kind(),
            fault.stage(),
        );
    }
}

#[test]
fn the_table_covers_every_tracked_adversarial_fixture() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixture_root().join("adversarial"))
        .expect("the adversarial directory is readable")
        .map(|entry| {
            entry
                .expect("a readable directory entry")
                .file_name()
                .into_string()
                .expect("fixture names are UTF-8")
        })
        .filter(|name| name.ends_with(".zip"))
        .collect();
    on_disk.sort();

    let mut tabulated: Vec<String> = ADVERSARIAL
        .iter()
        .map(|&(name, _, _)| name.to_owned())
        .collect();
    tabulated.sort();

    assert_eq!(
        tabulated, on_disk,
        "a fixture was added or removed without updating the conformance table"
    );
}

#[test]
fn refusing_the_same_package_twice_gives_the_same_answer() {
    for &(name, expected_kind, expected_stage) in ADVERSARIAL {
        let source = PackageSource::archive_file(adversarial(name));
        let first = load(&source).expect_err("refused");
        let second = load(&source).expect_err("refused");

        assert_eq!(first, second, "{name} refused differently on a repeat load");
        assert_eq!(
            (first.kind(), first.stage()),
            (expected_kind, expected_stage)
        );
    }
}

#[test]
fn an_archive_read_from_memory_is_refused_exactly_as_the_same_file_is() {
    for &(name, expected_kind, expected_stage) in ADVERSARIAL {
        let bytes = std::fs::read(adversarial(name)).expect("a readable fixture");
        let fault = load(&PackageSource::archive_bytes(bytes)).expect_err("refused");

        assert_eq!(
            (fault.kind(), fault.stage()),
            (expected_kind, expected_stage),
            "{name} was refused differently from memory than from a file"
        );
    }
}

#[test]
fn every_refusal_reports_an_actionable_public_status() {
    for &(name, expected_kind, _) in ADVERSARIAL {
        let expected_status = match expected_kind {
            AssetFaultKind::ArchiveLimit => Status::LimitExceeded,
            AssetFaultKind::UnsupportedSchemaVersion => Status::Unsupported,
            _ => Status::AssetInvalid,
        };
        let fault = load(&PackageSource::archive_file(adversarial(name))).expect_err("refused");

        assert_eq!(fault.status(), expected_status, "{name}");
    }
}

#[test]
fn a_refusal_never_names_the_content_that_caused_it() {
    // A diagnostic that quotes an attacker-controlled name is a diagnostic that
    // writes an attacker-controlled name into a log.
    for &(name, _, _) in ADVERSARIAL {
        let fault = load(&PackageSource::archive_file(adversarial(name))).expect_err("refused");
        let text = fault.to_string();

        assert!(
            !text.contains(".zip") && !text.contains('/') && !text.contains('\\'),
            "{name} produced a diagnostic quoting its source: {text}"
        );
    }
}

#[test]
fn the_entry_count_boundary_admits_the_ceiling_and_refuses_one_more() {
    let at_ceiling = usize::try_from(AssetLimits::MAX_ENTRY_COUNT).expect("fits");

    // The manifest plus enough empty entries to reach exactly the ceiling. An
    // archive of N entries is fully described by N, so this is built rather than
    // tracked; the fixture directory keeps only what its exact bytes are the
    // test of.
    let mut entries = vec![ArchiveEntry::file(
        mado_pilot_assets::MANIFEST_PATH,
        &empty_manifest(),
    )];
    for index in 1..at_ceiling {
        entries.push(ArchiveEntry::file(&format!("filler/{index:05}"), b""));
    }
    let package = load(&PackageSource::archive_bytes(write_archive(&entries, None)))
        .expect("an archive at the entry-count ceiling loads");
    assert_eq!(package.template_count(), 0);

    entries.push(ArchiveEntry::file("filler/one-too-many", b""));
    let fault = load(&PackageSource::archive_bytes(write_archive(&entries, None)))
        .expect_err("one entry above the ceiling is refused");

    assert_eq!(fault.kind(), AssetFaultKind::ArchiveLimit);
    assert_eq!(
        fault.stage(),
        LoadStage::DirectoryPreParse,
        "the count must be read from the trailer, before the central directory \
         is materialized"
    );
}

#[test]
fn a_recorded_count_above_the_ceiling_is_refused_before_the_directory_is_opened() {
    // The recorded count disagrees with the two records actually present, which
    // is what makes this a pre-parse test rather than an open test: an
    // implementation that trusted `ZipArchive` to count would reach the central
    // directory first.
    let archive = write_archive(
        &[
            ArchiveEntry::file(mado_pilot_assets::MANIFEST_PATH, &empty_manifest()),
            ArchiveEntry::file("filler/0", b""),
        ],
        Some(60_000),
    );
    let fault = load(&PackageSource::archive_bytes(archive)).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::ArchiveLimit);
    assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
}

#[test]
fn source_bytes_above_the_configured_limit_are_refused_before_anything_is_parsed() {
    // Crossing the tracked ceiling would need an archive larger than the ceiling
    // itself, so the check is measured against a lowered limit and a synthetic
    // buffer instead. The comparison under test is the same one either way.
    let limits = AssetLimits::ceiling()
        .with_max_total_compressed_bytes(64)
        .expect("below the ceiling");
    let archive = write_archive(
        &[ArchiveEntry::file(
            mado_pilot_assets::MANIFEST_PATH,
            &empty_manifest(),
        )],
        None,
    );
    assert!(archive.len() > 64, "the synthetic archive must cross it");

    let fault = PackageLoader::with_limits(limits)
        .load(
            &PackageSource::archive_bytes(archive),
            &OperationContext::new(),
        )
        .expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::ArchiveLimit);
    assert_eq!(fault.stage(), LoadStage::Source);
}

#[test]
fn every_tracked_fixture_still_hashes_to_its_recorded_checksum() {
    // A fixture change that is not accompanied by a re-measured evidence record
    // invalidates the gate resolution, so a silent edit has to be visible.
    let root = fixture_root();
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).expect("readable");
    let mut pinned: Vec<String> = Vec::new();

    for line in sums.lines().filter(|line| !line.trim().is_empty()) {
        let (expected, relative) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("unexpected checksum line: {line}"));
        // `.gitattributes` pins the tree to LF, so a checkout cannot introduce a
        // trailing `\r` here. Trimming anyway keeps a misconfigured clone failing
        // on the checksum it broke rather than on a file it cannot find.
        let relative = relative.trim().trim_start_matches("./");
        let bytes = std::fs::read(root.join(Path::new(relative)))
            .unwrap_or_else(|_| panic!("SHA256SUMS names a missing file: {relative}"));

        assert_eq!(
            hex_sha256(&bytes),
            expected,
            "{relative} no longer matches the checksum the evidence was taken against"
        );
        pinned.push(relative.to_owned());
    }

    // Pinning what is listed is only half of it. A fixture added without a
    // checksum would be evidence nothing was ever measured against.
    pinned.sort();
    let mut present = fixture_files(&root, &root);
    present.sort();

    assert_eq!(
        pinned, present,
        "every fixture must be pinned, and every pin must name a fixture"
    );
}

/// Returns every fixture file below `directory`, relative to `root`.
///
/// `SHA256SUMS` cannot pin itself, and the README describes the fixtures rather
/// than being one.
fn fixture_files(root: &Path, directory: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).expect("a readable fixture directory") {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            found.extend(fixture_files(root, &path));
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("every fixture is below the root")
            .to_str()
            .expect("fixture paths are UTF-8")
            .replace('\\', "/");
        if relative != "SHA256SUMS" && relative != "README.md" {
            found.push(relative);
        }
    }
    found
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
