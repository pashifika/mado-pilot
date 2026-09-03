//! Qualification-only access to facade-created provider ownership records.
//!
//! The public facade owns its concrete provider, so a native benchmark cannot
//! otherwise prove that a discovered opaque target belongs to its authenticated
//! fixture process. This weak registry exposes only that boolean observation and
//! retains neither engines nor providers.

use std::sync::{Arc, LazyLock, Mutex, Weak};

use mado_pilot_core::TargetId;

use crate::MacosCaptureProvider;
use crate::fixture_control::AuthenticatedFixtureProcess;

static PROVIDERS: LazyLock<Mutex<Vec<Weak<MacosCaptureProvider>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Registers one provider created by the existing facade constructor.
///
/// Dead providers are removed on every registration. The registry stores only
/// weak references and therefore cannot extend a session, engine, or native
/// resource lifetime.
pub fn register_provider(provider: &Arc<MacosCaptureProvider>) {
    let mut providers = PROVIDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    providers.retain(|candidate| candidate.strong_count() != 0);
    providers.push(Arc::downgrade(provider));
}

/// Returns whether `target` belongs to the authenticated live fixture process.
///
/// Target identities are engine-local, so at most the facade-created provider
/// that issued the identity can hold a matching snapshot record.
#[must_use]
pub fn target_has_authenticated_owner(
    target: TargetId,
    process: AuthenticatedFixtureProcess,
) -> bool {
    let mut providers = PROVIDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut authenticated = false;
    providers.retain(|candidate| {
        let Some(provider) = candidate.upgrade() else {
            return false;
        };
        authenticated |= provider.fixture_target_has_authenticated_owner(target, |owner| {
            process.matches_live_owner(owner)
        });
        true
    });
    authenticated
}
