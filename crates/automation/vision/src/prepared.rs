//! An immutable template a backend has already compiled.
//!
//! Preparation is separated from matching so the cost is paid once and reused,
//! and so a caller holds something it can pass around without holding the
//! backend's internals. What it holds is a reference-counted payload the
//! backend understands and nothing else does, plus the public metadata the
//! matcher needs to bound and correlate a result.

use std::fmt;
use std::sync::Arc;

use mado_pilot_core::PixelExtent;

use crate::backend::TemplatePayload;
use crate::template::{MatchDefaults, TemplateId, TemplateSource};

/// The identity of the backend that produced a prepared template.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackendId(Arc<str>);

impl BackendId {
    /// Builds an identity.
    #[must_use]
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    /// Returns the identity as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BackendId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A template compiled by one backend, ready to be matched repeatedly.
///
/// Cloning shares the compiled state. Nothing about the value changes after
/// construction, so two requests using the same prepared template are searching
/// for the same thing however far apart they are.
#[derive(Clone)]
pub struct PreparedTemplate {
    id: TemplateId,
    backend: BackendId,
    extent: PixelExtent,
    defaults: MatchDefaults,
    payload: Arc<dyn TemplatePayload>,
}

impl PreparedTemplate {
    /// Builds a prepared template around a backend's compiled `payload`.
    ///
    /// Called by a backend from its own `prepare`. The extent and defaults are
    /// copied from `source` rather than supplied separately, so a prepared
    /// template cannot disagree with the template it came from.
    #[must_use]
    pub fn new(
        backend: BackendId,
        source: &TemplateSource,
        payload: Arc<dyn TemplatePayload>,
    ) -> Self {
        Self {
            id: source.id().clone(),
            backend,
            extent: source.extent(),
            defaults: source.defaults(),
            payload,
        }
    }

    /// Returns the template's identity.
    #[must_use]
    pub const fn id(&self) -> &TemplateId {
        &self.id
    }

    /// Returns the backend that compiled this template.
    #[must_use]
    pub const fn backend(&self) -> &BackendId {
        &self.backend
    }

    /// Returns the extent every match of this template occupies.
    #[must_use]
    pub const fn extent(&self) -> PixelExtent {
        self.extent
    }

    /// Returns the matching options the template was authored with.
    #[must_use]
    pub const fn defaults(&self) -> MatchDefaults {
        self.defaults
    }

    /// Returns the backend's compiled state, for that backend to downcast.
    #[must_use]
    pub fn payload(&self) -> &dyn TemplatePayload {
        self.payload.as_ref()
    }
}

impl fmt::Debug for PreparedTemplate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The payload is deliberately summarized rather than printed: a
        // compiled template is pixels, and ordinary diagnostics exclude those.
        formatter
            .debug_struct("PreparedTemplate")
            .field("id", &self.id)
            .field("backend", &self.backend)
            .field("extent", &self.extent)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::sync::Arc;

    use mado_pilot_core::{CoordinateSpace, PixelExtent};

    use super::{BackendId, PreparedTemplate};
    use crate::backend::TemplatePayload;
    use crate::template::{
        MatchDefaults, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
    };

    #[derive(Debug)]
    struct Payload(#[expect(dead_code, reason = "proves the downcast reached this type")] u32);

    impl TemplatePayload for Payload {
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn source() -> TemplateSource {
        TemplateSource::new(TemplateSourceRequest {
            id: TemplateId::new("t").expect("non-empty"),
            encoding: TemplateEncoding::Png,
            extent: PixelExtent::new(16, 8),
            space: CoordinateSpace::CapturePixels,
            defaults: MatchDefaults::new(0.7, 3).expect("valid"),
            content: Arc::from([0x89, b'P', b'N', b'G'].as_slice()),
        })
        .expect("valid")
    }

    #[test]
    fn a_prepared_template_copies_its_sources_public_metadata() {
        let prepared = PreparedTemplate::new(
            BackendId::new("controlled"),
            &source(),
            Arc::new(Payload(7)),
        );

        assert_eq!(prepared.id().as_str(), "t");
        assert_eq!(prepared.extent(), PixelExtent::new(16, 8));
        assert_eq!(prepared.defaults().max_results(), 3);
        assert_eq!(prepared.backend().as_str(), "controlled");
    }

    #[test]
    fn the_owning_backend_can_downcast_its_payload() {
        let prepared = PreparedTemplate::new(
            BackendId::new("controlled"),
            &source(),
            Arc::new(Payload(7)),
        );

        assert!(
            prepared
                .payload()
                .as_any()
                .downcast_ref::<Payload>()
                .is_some()
        );
        assert!(prepared.payload().as_any().downcast_ref::<u64>().is_none());
    }

    #[test]
    fn debug_output_does_not_include_the_compiled_payload() {
        let prepared = PreparedTemplate::new(
            BackendId::new("controlled"),
            &source(),
            Arc::new(Payload(7)),
        );

        let text = format!("{prepared:?}");
        assert!(text.contains("PreparedTemplate"));
        assert!(!text.contains("Payload"));
    }
}
