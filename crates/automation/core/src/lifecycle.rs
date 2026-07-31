//! Where a session is between open and drained.
//!
//! Capture streams and input controllers answer the same three-state question, so
//! it is asked once here. Both close the same way: admission stops first, work
//! already admitted drains, and close is idempotent.

use std::fmt;

/// Where a stream or controller is in its lifecycle.
///
/// [`Lifecycle::Closing`] is an ordinary reachable state rather than a transient
/// one — a close whose operation is cancelled or already past its deadline leaves
/// a session there — so a caller that asked only whether close had *finished*
/// would keep handing work to a session that has stopped accepting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lifecycle {
    /// Publishing or accepting work.
    Open,
    /// Close has begun. New work is refused; admitted work is unwinding.
    Closing,
    /// Closed and drained.
    Closed,
}

impl Lifecycle {
    /// Reports whether new work may still be admitted.
    #[must_use]
    pub const fn accepts_work(self) -> bool {
        matches!(self, Lifecycle::Open)
    }

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Lifecycle::Open => "open",
            Lifecycle::Closing => "closing",
            Lifecycle::Closed => "closed",
        }
    }
}

impl fmt::Display for Lifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Lifecycle;

    #[test]
    fn only_an_open_session_accepts_work() {
        assert!(Lifecycle::Open.accepts_work());
        assert!(
            !Lifecycle::Closing.accepts_work(),
            "closing is where a cancelled close leaves a session"
        );
        assert!(!Lifecycle::Closed.accepts_work());
    }

    #[test]
    fn states_have_stable_slugs() {
        assert_eq!(Lifecycle::Open.to_string(), "open");
        assert_eq!(Lifecycle::Closing.to_string(), "closing");
        assert_eq!(Lifecycle::Closed.to_string(), "closed");
    }
}
