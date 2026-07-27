//! Turning a recorded entry name into a safe relative package path.
//!
//! The rules are transcribed from `docs/evidence/g-014/probe.md`, because the
//! adversarial fixtures were measured against exactly them. Changing one here
//! changes what the tracked evidence is evidence of.
//!
//! Nothing in this module touches the filesystem. A package path is an
//! identifier inside a package, never a location to open: an archive entry is
//! read in place by index, and a directory entry is resolved by joining
//! validated components onto the root the caller named.

use std::fmt;

/// A validated relative path inside a package.
///
/// Two names that normalize to the same `PackagePath` are the same entry
/// spelled two ways, which is what makes a duplicate detectable rather than
/// merely malformed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackagePath(String);

impl PackagePath {
    /// Normalizes a recorded entry name, or returns `None` when it is unsafe.
    ///
    /// A name is rejected when it is not valid UTF-8, is empty, contains a NUL
    /// or a backslash, begins with `/`, carries a drive prefix (an ASCII letter
    /// followed by `:`), ends with `/`, or contains a `..` segment. Otherwise
    /// `.` and empty segments are dropped and the remaining segments are joined
    /// with `/`.
    ///
    /// Collapsing `.` and empty segments rather than rejecting them is what
    /// makes a duplicate detectable: `templates/button.png` and
    /// `./templates//button.png` are the same package path spelled two ways, and
    /// the second must be reported as a duplicate rather than as a malformed
    /// name.
    ///
    /// A backslash is rejected rather than treated as a separator so that one
    /// archive produces one outcome on both release targets. Windows would
    /// otherwise read `a\b` as two components and macOS as one filename.
    #[must_use]
    pub fn normalize(raw: &[u8]) -> Option<Self> {
        let name = std::str::from_utf8(raw).ok()?;

        if name.is_empty()
            || name.contains('\0')
            || name.contains('\\')
            || name.starts_with('/')
            || name.ends_with('/')
            || has_drive_prefix(name)
        {
            return None;
        }

        let mut normalized = String::with_capacity(name.len());
        for segment in name.split('/') {
            if segment == ".." {
                return None;
            }
            if segment.is_empty() || segment == "." {
                continue;
            }
            if !normalized.is_empty() {
                normalized.push('/');
            }
            normalized.push_str(segment);
        }

        if normalized.is_empty() {
            return None;
        }
        Some(Self(normalized))
    }

    /// Returns the normalized path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackagePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn has_drive_prefix(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::PackagePath;

    fn normalized(raw: &str) -> Option<String> {
        PackagePath::normalize(raw.as_bytes()).map(|path| path.as_str().to_owned())
    }

    #[test]
    fn a_plain_relative_name_survives_unchanged() {
        assert_eq!(
            normalized("templates/button.png"),
            Some("templates/button.png".to_owned())
        );
        assert_eq!(
            normalized("madopilot-package.json"),
            Some("madopilot-package.json".to_owned())
        );
    }

    #[test]
    fn redundant_segments_collapse_so_a_duplicate_is_detectable() {
        assert_eq!(
            normalized("./templates//button.png"),
            Some("templates/button.png".to_owned())
        );
        assert_eq!(
            normalized("templates/./button.png"),
            Some("templates/button.png".to_owned())
        );
    }

    #[test]
    fn absolute_and_rooted_names_are_rejected() {
        assert_eq!(normalized("/etc/shadow"), None);
        assert_eq!(normalized("//server/share/x.png"), None);
        assert_eq!(normalized("C:/Windows/x.png"), None);
        assert_eq!(normalized("c:x.png"), None);
    }

    #[test]
    fn traversal_is_rejected_wherever_it_appears() {
        assert_eq!(normalized("../x.png"), None);
        assert_eq!(normalized("templates/../../x.png"), None);
        assert_eq!(normalized("templates/.."), None);
    }

    #[test]
    fn a_name_that_would_read_differently_per_target_is_rejected() {
        assert_eq!(normalized("templates\\button.png"), None);
        assert_eq!(normalized("\\\\server\\share\\x.png"), None);
        assert_eq!(normalized("templates/button.png\0.txt"), None);
        assert_eq!(PackagePath::normalize(&[0xff, 0xfe, b'a']), None);
    }

    #[test]
    fn a_directory_entry_is_not_a_package_path() {
        assert_eq!(normalized("templates/"), None);
        assert_eq!(normalized("."), None);
        assert_eq!(normalized(""), None);
    }

    #[test]
    fn a_dot_prefixed_file_name_is_not_a_traversal() {
        assert_eq!(
            normalized("templates/.keep"),
            Some("templates/.keep".to_owned())
        );
        assert_eq!(normalized("...png"), Some("...png".to_owned()));
    }

    #[test]
    fn nested_relative_names_keep_their_components() {
        assert_eq!(
            normalized("templates/sub/button.png"),
            Some("templates/sub/button.png".to_owned())
        );
    }
}
