//! The composition of the capture, asset, and vision contracts.
//!
//! An engine is the one place that holds all three at once, and every operation
//! here exists because it spans more than one of them. Nothing in this file
//! names a concrete adapter: which capture provider and which matching backend
//! an engine was built from is the facade's decision, and an engine cannot
//! observe or change it.

use std::sync::Arc;

use mado_pilot_assets::{AssetFault, AssetLimits, AssetPackage, PackageLoader, PackageSource};
use mado_pilot_capture::{CaptureProvider, OpenRequest, TargetDescription};
use mado_pilot_core::{EngineId, Error, Operation, OperationContext, TargetId};
use mado_pilot_vision::{BackendDescriptor, Matcher, PreparedTemplate, TemplateSource};

use crate::session::Session;

/// The contract dependencies one engine orchestrates.
///
/// This is the seam a composition root wires; it is not a plugin registry. The
/// facade is the only package that fills it in, because naming a concrete
/// adapter is the facade's responsibility and nobody else's.
#[derive(Debug)]
pub struct EngineWiring {
    /// The identity every target and stream this engine accepts is scoped to.
    pub engine: EngineId,
    /// The capture adapter that discovers targets and opens sessions.
    pub capture: Arc<dyn CaptureProvider>,
    /// The matcher this engine prepares templates and searches frames through.
    pub matcher: Matcher,
    /// The loader every asset package this engine loads is validated by.
    pub loader: PackageLoader,
}

/// An engine over one capture adapter and one matching backend.
#[derive(Debug)]
pub struct Engine {
    engine: EngineId,
    capture: Arc<dyn CaptureProvider>,
    matcher: Matcher,
    loader: PackageLoader,
}

impl Engine {
    /// Builds an engine from wired contract dependencies.
    #[must_use]
    pub fn new(wiring: EngineWiring) -> Self {
        Self {
            engine: wiring.engine,
            capture: wiring.capture,
            matcher: wiring.matcher,
            loader: wiring.loader,
        }
    }

    /// Returns the identity that scopes this engine's targets and streams.
    ///
    /// A target identity issued by another engine is refused rather than
    /// resolved, so a caller holding two engines can tell which one an identity
    /// belongs to before it asks.
    #[must_use]
    pub const fn id(&self) -> EngineId {
        self.engine
    }

    /// Returns the matching backend that will produce every score.
    #[must_use]
    pub fn backend(&self) -> BackendDescriptor {
        self.matcher.descriptor()
    }

    /// Returns the limits every package loaded through this engine is held to.
    #[must_use]
    pub const fn limits(&self) -> AssetLimits {
        self.loader.limits()
    }

    /// Lists the targets this engine's capture adapter can currently capture.
    ///
    /// # Errors
    ///
    /// Returns a capture failure when the configured source cannot be read, and
    /// the operation's terminal outcome when cancellation or the deadline wins.
    pub fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>, Error> {
        self.capture.discover(operation)
    }

    /// Opens a session for `target` that can search its own frames.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome for a target this engine did not
    /// issue, an unsupported outcome for a required option the adapter cannot
    /// honor, and the operation's terminal outcome when cancellation or the
    /// deadline wins.
    pub fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Session, Error> {
        let attempt = Operation::admit(operation)?;
        let capture = self.capture.open(target, request, operation)?;
        Ok(attempt.commit(Session::new(capture, self.matcher.clone()))?)
    }

    /// Loads and validates an asset package from `source`.
    ///
    /// This reports the asset package's own typed fault rather than the shared
    /// error, because which rule a package broke and how far loading had got
    /// are both part of that contract, and flattening them into a status would
    /// leave a caller reading the message to tell a bad hash from a bad path.
    ///
    /// # Errors
    ///
    /// Returns an [`AssetFault`] carrying the rule that was broken and the
    /// stage that caught it.
    pub fn load_package(
        &self,
        source: &PackageSource,
        operation: &OperationContext,
    ) -> Result<AssetPackage, AssetFault> {
        self.loader.load(source, operation)
    }

    /// Compiles a template source for this engine's backend.
    ///
    /// A template source does not have to come from an asset package: the
    /// vision contract accepts one built from bytes a caller read itself, and
    /// an engine that only accepted packaged templates would make the manifest
    /// mandatory for a caller that has no use for it. The packaged form is
    /// [`Engine::prepare_from_package`]; the two differ in where the source
    /// comes from and in nothing else.
    ///
    /// # Errors
    ///
    /// Returns the backend's typed preparation failure, and the operation's
    /// terminal outcome when cancellation or the deadline wins.
    pub fn prepare_template(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate, Error> {
        self.matcher.prepare(source, operation)
    }

    /// Compiles the template `id` names in `package` for this engine's backend.
    ///
    /// Resolution and compilation run under one operation, so a request that
    /// loses its race is refused rather than half-applied, and an identity the
    /// package does not contain is refused before the backend is asked to do
    /// any work at all.
    ///
    /// The asset detail behind a missing identity is available from
    /// [`AssetPackage::resolve_template`] for a caller that needs to
    /// distinguish it from another asset failure by more than its status.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome when `package` contains no template
    /// with that identity — a package that loaded is valid, and asking it for
    /// something it never declared is the caller's mistake — the backend's
    /// typed preparation failure, and the operation's terminal outcome when
    /// cancellation or the deadline wins.
    pub fn prepare_from_package(
        &self,
        package: &AssetPackage,
        id: &str,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate, Error> {
        let mut attempt = Operation::admit(operation)?;
        let source = package.resolve_template(id)?;
        attempt.checkpoint()?;

        let prepared = self.matcher.prepare(&source, operation)?;
        Ok(attempt.commit(prepared)?)
    }
}
