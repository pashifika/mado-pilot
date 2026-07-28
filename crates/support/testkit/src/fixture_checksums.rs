//! Checking that a tracked fixture directory still is what a measurement was
//! taken against.
//!
//! A `SHA256SUMS` file that nothing reads is documentation, not a pin. What
//! makes it a pin is a check in *both* directions: every file it lists still
//! hashes to what it says, and every file present is listed. The second half is
//! the one that is easy to leave out and the one that matters when a fixture is
//! added — an unlisted file is one no evidence was ever taken against.
//!
//! `mado-pilot-assets` and `mado-pilot` each make this check for their own
//! fixture set, in their own test, against
//! `mado_pilot_assets::ContentDigest`. This exists for a fixture set whose
//! crate has no reason to depend on the asset vocabulary: the replay adapter
//! depends on the core and capture contracts and nothing else, and adding an
//! asset dependency to reach a digest constructor would trade a small
//! duplication for an architectural one.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Files that describe a fixture set rather than being part of it.
///
/// `SHA256SUMS` cannot pin itself, and a README describes the fixtures.
const NOT_A_FIXTURE: [&str; 2] = ["SHA256SUMS", "README.md"];

/// Checks every file under `root` against the `SHA256SUMS` beside them.
///
/// # Panics
///
/// Panics when the checksum file is unreadable or malformed, when a listed file
/// is missing or no longer matches its digest, or when a file is present that
/// the checksum file does not list.
pub fn verify(root: &Path) {
    let listing = root.join("SHA256SUMS");
    let text = std::fs::read_to_string(&listing)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", listing.display()));

    let mut pinned: Vec<String> = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (expected, relative) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("unexpected checksum line: {line}"));
        // `.gitattributes` pins the tree to LF, so a checkout cannot introduce a
        // trailing `\r` here. Trimming anyway keeps a misconfigured clone
        // failing on the checksum it broke rather than on a file it cannot find.
        let relative = relative.trim().trim_start_matches("./");
        let bytes = std::fs::read(root.join(relative))
            .unwrap_or_else(|_| panic!("SHA256SUMS names a missing file: {relative}"));

        assert_eq!(
            hex_sha256(&bytes),
            expected,
            "{relative} no longer matches the checksum the evidence was taken against"
        );
        pinned.push(relative.to_owned());
    }

    pinned.sort();
    let mut present = files(root, root);
    present.sort();

    assert_eq!(
        pinned, present,
        "every fixture must be pinned, and every pin must name a fixture"
    );
}

/// Returns every pinnable file below `directory`, relative to `root`.
fn files(root: &Path, directory: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).expect("a readable fixture directory") {
        let path: PathBuf = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            found.extend(files(root, &path));
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("every fixture is below the root")
            .to_str()
            .expect("fixture paths are UTF-8")
            .replace('\\', "/");
        if !NOT_A_FIXTURE.contains(&relative.as_str()) {
            found.push(relative);
        }
    }

    found
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
