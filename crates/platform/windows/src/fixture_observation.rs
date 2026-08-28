//! Qualification-only access to facade-created provider ownership records.

use std::sync::{Arc, LazyLock, Mutex, Weak};

use mado_pilot_core::TargetId;

use crate::WindowsCaptureProvider;

static PROVIDERS: LazyLock<Mutex<Vec<Weak<WindowsCaptureProvider>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Registers one provider created by the existing facade constructor.
///
/// Only weak references are retained, and dead providers are pruned on every
/// access so instrumentation cannot extend native resource lifetimes.
pub fn register_provider(provider: &Arc<WindowsCaptureProvider>) {
    let mut providers = PROVIDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    providers.retain(|candidate| candidate.strong_count() != 0);
    providers.push(Arc::downgrade(provider));
}

/// Returns whether the provider snapshot that issued `target` binds it to the
/// live repository fixture process.
#[must_use]
pub fn target_has_process(target: TargetId, process_id: u32) -> bool {
    let mut providers = PROVIDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut authenticated = false;
    providers.retain(|candidate| {
        let Some(provider) = candidate.upgrade() else {
            return false;
        };
        authenticated |= provider.fixture_target_has_process(target, process_id);
        true
    });
    authenticated
}
