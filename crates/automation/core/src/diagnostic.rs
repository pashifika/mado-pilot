//! Redacted diagnostics: what a native failure may say about the desktop.
//!
//! A discovery, capability, or permission failure has to be actionable without
//! describing the user's desktop. The two are separated here structurally rather
//! than by review: a diagnostic carries a category, an optional numeric platform
//! code, and context text the Adapter wrote itself. There is no field an
//! operating-system string, a window title, a file path, or a pixel can reach.

use std::fmt;

/// What kind of problem a redacted diagnostic reports.
///
/// The category is what a caller acts on: retry, ask the user for a permission,
/// choose another target, or give up. It is deliberately coarse, because a finer
/// category would start describing the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticCategory {
    /// The operating system refused the operation for want of authorization.
    PermissionDenied,
    /// A non-prompting probe could not establish authorization either way.
    PermissionUndetermined,
    /// The platform, build, or host does not offer the capability at all.
    CapabilityUnavailable,
    /// The target existed when it was discovered and does not now.
    TargetLost,
    /// The platform reported a failure that none of the above explains.
    PlatformFailure,
    /// The request or the Adapter's own configuration is inconsistent.
    Configuration,
}

impl DiagnosticCategory {
    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DiagnosticCategory::PermissionDenied => "permission_denied",
            DiagnosticCategory::PermissionUndetermined => "permission_undetermined",
            DiagnosticCategory::CapabilityUnavailable => "capability_unavailable",
            DiagnosticCategory::TargetLost => "target_lost",
            DiagnosticCategory::PlatformFailure => "platform_failure",
            DiagnosticCategory::Configuration => "configuration",
        }
    }
}

impl fmt::Display for DiagnosticCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One numeric code as the platform reported it, with the space it came from.
///
/// The namespace is what makes the number interpretable: `0x80070005` means
/// something in `HRESULT` and nothing at all on its own. Both parts are needed
/// to look a failure up, and neither carries desktop content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlatformCode {
    namespace: &'static str,
    code: i64,
}

impl PlatformCode {
    /// Records `code` from the platform error space named `namespace`.
    ///
    /// `namespace` is an Adapter literal such as `"hresult"`, `"win32"`,
    /// `"osstatus"`, or `"errno"`, never text an operating system returned.
    #[must_use]
    pub const fn new(namespace: &'static str, code: i64) -> Self {
        Self { namespace, code }
    }

    /// Returns the error space the code belongs to.
    #[must_use]
    pub const fn namespace(self) -> &'static str {
        self.namespace
    }

    /// Returns the platform's own numeric code.
    #[must_use]
    pub const fn code(self) -> i64 {
        self.code
    }
}

impl fmt::Display for PlatformCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.namespace, self.code)
    }
}

/// An actionable native diagnostic that cannot carry desktop content.
///
/// `context` is `&'static str` on purpose, and that is the whole redaction
/// mechanism: a string literal exists in the Adapter's source, so it can be
/// reviewed once, while an owned string would let a window title, a recognized
/// line of text, or an operating-system message reach a log by accident. An
/// Adapter that needs to report a value the operating system produced adds a
/// typed field for exactly that value instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RedactedDiagnostic {
    category: DiagnosticCategory,
    platform: Option<PlatformCode>,
    context: &'static str,
}

impl RedactedDiagnostic {
    /// Reports `category` with no platform code and no further context.
    #[must_use]
    pub const fn new(category: DiagnosticCategory) -> Self {
        Self {
            category,
            platform: None,
            context: "",
        }
    }

    /// Adds the platform's own error code.
    #[must_use]
    pub const fn with_platform(mut self, platform: PlatformCode) -> Self {
        self.platform = Some(platform);
        self
    }

    /// Adds Adapter-authored context, such as which native call refused.
    #[must_use]
    pub const fn with_context(mut self, context: &'static str) -> Self {
        self.context = context;
        self
    }

    /// Returns the category a caller acts on.
    #[must_use]
    pub const fn category(self) -> DiagnosticCategory {
        self.category
    }

    /// Returns the platform's error code, when the Adapter recorded one.
    #[must_use]
    pub const fn platform(self) -> Option<PlatformCode> {
        self.platform
    }

    /// Returns the Adapter-authored context, or an empty string.
    #[must_use]
    pub const fn context(self) -> &'static str {
        self.context
    }
}

impl fmt::Display for RedactedDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.category)?;
        if let Some(platform) = self.platform {
            write!(formatter, " ({platform})")?;
        }
        if !self.context.is_empty() {
            write!(formatter, ": {}", self.context)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticCategory, PlatformCode, RedactedDiagnostic};

    #[test]
    fn a_category_alone_is_a_complete_diagnostic() {
        let diagnostic = RedactedDiagnostic::new(DiagnosticCategory::PermissionUndetermined);

        assert_eq!(
            diagnostic.category(),
            DiagnosticCategory::PermissionUndetermined
        );
        assert_eq!(diagnostic.platform(), None);
        assert_eq!(diagnostic.context(), "");
        assert_eq!(diagnostic.to_string(), "permission_undetermined");
    }

    #[test]
    fn a_platform_code_carries_the_space_that_makes_it_readable() {
        let diagnostic = RedactedDiagnostic::new(DiagnosticCategory::PlatformFailure)
            .with_platform(PlatformCode::new("hresult", 0x8007_0005))
            .with_context("opening the capture item");

        assert_eq!(
            diagnostic.platform().map(PlatformCode::namespace),
            Some("hresult")
        );
        assert_eq!(
            diagnostic.platform().map(PlatformCode::code),
            Some(0x8007_0005)
        );
        assert_eq!(
            diagnostic.to_string(),
            "platform_failure (hresult:2147942405): opening the capture item"
        );
    }

    #[test]
    fn categories_have_stable_slugs() {
        for (category, slug) in [
            (DiagnosticCategory::PermissionDenied, "permission_denied"),
            (
                DiagnosticCategory::PermissionUndetermined,
                "permission_undetermined",
            ),
            (
                DiagnosticCategory::CapabilityUnavailable,
                "capability_unavailable",
            ),
            (DiagnosticCategory::TargetLost, "target_lost"),
            (DiagnosticCategory::PlatformFailure, "platform_failure"),
            (DiagnosticCategory::Configuration, "configuration"),
        ] {
            assert_eq!(category.as_str(), slug);
            assert_eq!(category.to_string(), slug);
        }
    }
}
