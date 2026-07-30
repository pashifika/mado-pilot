//! Reachable points at which a test can make the boundary panic.
//!
//! Panic containment is only believable if a panic actually happens, and a
//! panic that only happens in a test that calls `panic!` itself tests
//! `catch_unwind` rather than the boundary. These sites are inside the real
//! entries, on the real paths, so an armed test exercises the same code an
//! accident would.
//!
//! Two sites are enough for what the contract distinguishes:
//! [`Site::Entry`] fires after every owned output has been set to its failure
//! state and before any work, and [`Site::AfterTemporary`] fires after temporary
//! storage exists and before success commits.
//!
//! Outside `cfg(test)` [`reach`] compiles to nothing. Arming is a test-only
//! capability and there is no way to reach it from C, from another crate, or
//! from a release build.

/// A point inside an entry where an armed test panics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Site {
    /// After outputs are in their failure state, before any work.
    Entry,
    /// After temporary storage exists, before success commits.
    AfterTemporary,
}

#[cfg(not(test))]
#[inline(always)]
pub(crate) fn reach(_site: Site) {}

#[cfg(test)]
pub(crate) use testing::{armed, reach};

#[cfg(test)]
mod testing {
    use std::cell::Cell;

    use super::Site;

    thread_local! {
        static ARMED: Cell<Option<Site>> = const { Cell::new(None) };
    }

    /// Panics when `site` is the armed site, and disarms so one arming produces
    /// exactly one panic.
    pub(crate) fn reach(site: Site) {
        let fire = ARMED.with(|armed| {
            if armed.get() == Some(site) {
                armed.set(None);
                true
            } else {
                false
            }
        });

        assert!(!fire, "test hook: deliberate panic at {site:?}");
    }

    /// Runs `body` with `site` armed, and disarms afterwards.
    pub(crate) fn armed<T>(site: Site, body: impl FnOnce() -> T) -> T {
        ARMED.with(|armed| armed.set(Some(site)));
        let outcome = body();
        ARMED.with(|armed| armed.set(None));

        outcome
    }
}
