//! MadoPilot OpenCV CPU vision backend.
//!
//! # Responsibility
//!
//! This package implements the `mado-pilot-vision` matching backend contract on
//! top of OpenCV's `matchTemplate`. It decodes template content, converts a
//! searched region into the layout OpenCV works in, correlates the two, and
//! reports non-overlapping candidates.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, capture, and vision contract
//! packages. The capture dependency is the same contract-to-contract edge the
//! vision package has: a backend is handed a capture-owned CPU mapping and has
//! to name that type. No contract package depends on this one, and the public
//! facade is what wires it in.
//!
//! No OpenCV type appears in any signature this package exports. A decoded
//! template travels as an opaque payload inside a
//! [`PreparedTemplate`](mado_pilot_vision::PreparedTemplate), which the matcher
//! attributes to this backend before anything downcasts it.
//!
//! # What it decides, and what it does not
//!
//! Region resolution, public score validation, thresholding, canonical ordering,
//! overlap suppression, the result limit, and the result envelope belong to
//! `mado-pilot-vision`'s matcher. This adapter finds candidates; it decides
//! nothing about what a match *is*.
//!
//! # The Phase 1 matching profile
//!
//! Three-channel BGR, correlated with `TM_CCOEFF_NORMED`, with the negative half
//! of the correlation range clamped to "no match" and candidates extracted as
//! non-overlapping peaks. Each of those is a decision with a recorded reason and
//! a rejected alternative; see
//! `docs/adr/0003-opencv-matching-profile-and-public-score.md`.
//!
//! # Development prerequisite
//!
//! Building this package needs an OpenCV 4 development installation and a
//! libclang the binding generator can load. Phase 1 treats both as development
//! prerequisites and makes no claim about what a release ships; see
//! `docs/third-party-dependencies.md` and gate `G-007` in
//! `docs/validation-gates.md`.
//!
//! # Implementation status
//!
//! Phase 1, complete, implemented for the Phase 1 profile. Preprocessing
//! descriptors, scaling, masked matching, rotation, pyramids, and GPU execution
//! are not implemented and are not reserved as empty seams.
//!
//! **The public names here are reviewed, not yet stable.**
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md` records the
//! review that settled them and the policy that now applies: renaming or
//! removing one is a breaking change needing an ADR and a version bump, while
//! adding is free. The stability promise itself begins at 1.0.
//!
//! # Where to start
//!
//! Building this package already requires a usable OpenCV, so this example runs
//! rather than merely compiling: it is the shortest check that the installation
//! the crate was built against is the one it can also load.
//!
//! ```
//! use std::sync::Arc;
//!
//! use mado_pilot_backend_opencv::OpenCvBackend;
//! use mado_pilot_vision::{MatchBackend, Matcher};
//!
//! // Construction probes the linked library, so an unusable OpenCV is a status
//! // rather than a surprise at the first search.
//! let backend = OpenCvBackend::new()?;
//! let matcher = Matcher::new(Arc::new(backend) as Arc<dyn MatchBackend>);
//!
//! assert_eq!(matcher.descriptor().id(), mado_pilot_backend_opencv::BACKEND_ID);
//! # Ok::<(), mado_pilot_core::Error>(())
//! ```

mod backend;
mod candidates;
mod image;

pub use backend::{BACKEND_ID, OpenCvBackend};
