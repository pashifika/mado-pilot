//! Directory containment, and the entry types only a directory can carry.
//!
//! An archive entry cannot be a symbolic link on disk, a device node, or a file
//! whose name is not valid UTF-8. A directory can be all three, and Git cannot
//! track any of them portably — a symlink checked out on Windows without
//! developer mode becomes a regular file, which would make a tracked fixture
//! test nothing. So these cases are built here, at test time, by the change
//! that implements directory loading.
//!
//! The Unix-only cases are not gaps on Windows. A directory entry that is not a
//! regular file is classified as such by the walk and then refused by exactly
//! the same rule that refuses `entry-symlink.zip`, `entry-fifo.zip`, and
//! `entry-character-device.zip`, and those archives run on both release
//! targets.

#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

use mado_pilot_assets::{
    AssetFaultKind, AssetLimits, LoadStage, MANIFEST_PATH, PackageLoader, PackageSource,
};
#[cfg(unix)]
use mado_pilot_core::MonotonicInstant;
use mado_pilot_core::OperationContext;

mod support;

#[cfg(unix)]
use support::ReplacingClock;
use support::{PNG_SIGNATURE, TempDir, load, single_template_manifest};

#[test]
fn a_nested_directory_is_traversed_rather_than_treated_as_an_entry() {
    let temporary = TempDir::new("nested");
    let content = [PNG_SIGNATURE, b"deep"].concat();
    temporary.write(
        MANIFEST_PATH,
        &single_template_manifest("art/buttons/start.png", (12, 8), &content),
    );
    temporary.write("art/buttons/start.png", &content);

    let package = load(&PackageSource::directory(temporary.path())).expect("valid");
    let template = package.resolve_template("only").expect("declared");

    assert_eq!(template.content(), content.as_slice());
    assert_eq!(
        package.manifest().templates()[0].path().as_str(),
        "art/buttons/start.png",
        "package paths use forward slashes whatever the platform's separator is"
    );
}

#[test]
fn an_empty_directory_has_no_manifest() {
    let temporary = TempDir::new("empty");

    let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::MissingManifest);
    assert_eq!(fault.stage(), LoadStage::Manifest);
}

#[test]
fn a_source_that_is_not_a_directory_is_reported_as_unreadable() {
    let temporary = TempDir::new("not-a-directory");
    let file = temporary.write("plain.txt", b"not a package");

    let fault = load(&PackageSource::directory(&file)).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::SourceUnreadable);
    assert_eq!(fault.stage(), LoadStage::Source);
}

#[test]
fn an_external_hard_link_is_refused_as_a_non_regular_entry() {
    let package = TempDir::new("external-hard-link-package");
    let external = TempDir::new("external-hard-link-target");
    package.write(MANIFEST_PATH, &support::empty_manifest());
    let target = external.write("outside.bin", b"outside the package");
    std::fs::hard_link(&target, package.path().join("linked.bin"))
        .expect("the supported target filesystem can create hard links");

    let fault = load(&PackageSource::directory(package.path())).expect_err("refused");

    assert_eq!(fault.kind(), AssetFaultKind::UnsupportedEntryType);
    assert_eq!(fault.stage(), LoadStage::EntryMetadata);
}

#[cfg(unix)]
#[test]
fn a_directory_entry_replaced_between_identity_checks_is_refused() {
    let package = TempDir::new("directory-identity-swap");
    let staged = TempDir::new("directory-identity-replacement");
    let content = [PNG_SIGNATURE, b"stable"].concat();
    let manifest = single_template_manifest("template.png", (4, 4), &content);
    let target = package.write(MANIFEST_PATH, &manifest);
    package.write("template.png", &content);
    let replacement = staged.write("replacement.json", &manifest);

    // For this two-entry tree, read nine is the checkpoint between the first
    // identity-bearing open of the sorted manifest entry and its retained open.
    let clock = Arc::new(ReplacingClock::new(9, &target, replacement));
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(60)));

    let fault = PackageLoader::new()
        .load(&PackageSource::directory(package.path()), &context)
        .expect_err("an identity-changing replacement is refused");

    assert!(clock.replaced(), "the controlled replacement must have run");
    assert_eq!(fault.kind(), AssetFaultKind::SourceChanged);
    assert_eq!(fault.stage(), LoadStage::Source);
}

#[test]
fn directory_nodes_are_bounded_before_the_listing_is_collected() {
    let temporary = TempDir::new("directory-node-limit");
    temporary.write(MANIFEST_PATH, &support::empty_manifest());
    for name in ["empty-a", "empty-b"] {
        std::fs::create_dir(temporary.path().join(name)).expect("empty directory");
    }
    let limits = AssetLimits::ceiling()
        .with_max_entry_count(2)
        .expect("below the ceiling");

    let fault = PackageLoader::with_limits(limits)
        .load(
            &PackageSource::directory(temporary.path()),
            &OperationContext::new(),
        )
        .expect_err("structural nodes consume the traversal budget");

    assert_eq!(fault.kind(), AssetFaultKind::ArchiveLimit);
    assert_eq!(fault.stage(), LoadStage::Source);
}

#[test]
fn repeated_loads_of_the_same_invalid_tree_give_the_same_answer() {
    let temporary = TempDir::new("deterministic-refusal");
    let content = [PNG_SIGNATURE, b"x"].concat();
    temporary.write(
        MANIFEST_PATH,
        &single_template_manifest("missing.png", (4, 4), &content),
    );
    // Several unreferenced files, so directory iteration order has something to
    // vary. The walk sorts, so it cannot.
    for name in ["zzz.bin", "aaa.bin", "mmm.bin"] {
        temporary.write(name, b"filler");
    }

    let first = load(&PackageSource::directory(temporary.path())).expect_err("refused");
    let second = load(&PackageSource::directory(temporary.path())).expect_err("refused");

    assert_eq!(first, second);
    assert_eq!(first.kind(), AssetFaultKind::MissingEntry);
}

#[cfg(unix)]
mod unix_only {
    use std::os::unix::fs::symlink;

    use mado_pilot_assets::{AssetFaultKind, LoadStage, MANIFEST_PATH, PackageSource};

    use crate::support::{self, PNG_SIGNATURE, TempDir, load, single_template_manifest};

    #[test]
    fn a_symlink_is_refused_without_being_followed() {
        let temporary = TempDir::new("symlink-entry");
        let content = [PNG_SIGNATURE, b"real"].concat();
        temporary.write(
            MANIFEST_PATH,
            &single_template_manifest("templates/start.png", (4, 4), &content),
        );
        let real = temporary.write("templates/start.png", &content);
        symlink(&real, temporary.path().join("templates/alias.png")).expect("a writable symlink");

        let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

        assert_eq!(fault.kind(), AssetFaultKind::UnsupportedEntryType);
        assert_eq!(fault.stage(), LoadStage::EntryMetadata);
    }

    #[test]
    fn a_symlink_pointing_outside_the_package_is_refused_before_it_could_escape() {
        let temporary = TempDir::new("escaping-symlink");
        temporary.write(MANIFEST_PATH, &support::empty_manifest());
        symlink("/etc/passwd", temporary.path().join("escape")).expect("a writable symlink");

        let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

        assert_eq!(
            fault.kind(),
            AssetFaultKind::UnsupportedEntryType,
            "the link is refused as a type, so its target is never resolved"
        );
    }

    #[test]
    fn a_symlinked_subdirectory_is_refused_rather_than_walked_into() {
        // The dangerous shape: a link the walk could descend through and read a
        // whole tree outside the package from. It is classified by
        // `symlink_metadata`, which does not follow, so it never becomes a
        // directory to descend into.
        let temporary = TempDir::new("symlinked-subdirectory");
        temporary.write(MANIFEST_PATH, &support::empty_manifest());
        symlink("/etc", temporary.path().join("elsewhere")).expect("a writable symlink");

        let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

        assert_eq!(fault.kind(), AssetFaultKind::UnsupportedEntryType);
        assert_eq!(fault.stage(), LoadStage::EntryMetadata);
    }

    #[test]
    fn a_dangling_symlink_is_refused_as_a_type_rather_than_as_a_read_failure() {
        let temporary = TempDir::new("dangling-symlink");
        temporary.write(MANIFEST_PATH, &support::empty_manifest());
        symlink(
            temporary.path().join("nowhere"),
            temporary.path().join("dangling"),
        )
        .expect("a writable symlink");

        let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

        assert_eq!(fault.kind(), AssetFaultKind::UnsupportedEntryType);
    }

    #[test]
    fn a_backslash_in_a_file_name_is_refused() {
        // A name Windows cannot express and Unix can. Accepting it would give
        // one package two meanings, so it is refused on both.
        let temporary = TempDir::new("backslash-name");
        temporary.write(MANIFEST_PATH, &support::empty_manifest());
        temporary.write("templates\\start.png", PNG_SIGNATURE);

        let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

        assert_eq!(fault.kind(), AssetFaultKind::UnsafePath);
        assert_eq!(fault.stage(), LoadStage::EntryMetadata);
    }

    /// Neither release target can create this name — APFS and NTFS both refuse
    /// it — so the guard is defence in depth there and the tracked
    /// `path-non-utf8.zip` carries the equivalent archive coverage. Linux can
    /// create it, so the branch is exercised on the workspace CI job that runs
    /// there rather than left untested everywhere.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_file_name_that_is_not_utf8_is_refused_rather_than_skipped() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let temporary = TempDir::new("non-utf8-name");
        temporary.write(MANIFEST_PATH, &support::empty_manifest());
        let name = OsStr::from_bytes(&[b'b', b'a', 0xff, b'd']);
        std::fs::write(temporary.path().join(name), b"x").expect("a writable file");

        let fault = load(&PackageSource::directory(temporary.path())).expect_err("refused");

        assert_eq!(
            fault.kind(),
            AssetFaultKind::UnsafePath,
            "a package that silently lost a file is worse than one that failed to load"
        );
    }
}
