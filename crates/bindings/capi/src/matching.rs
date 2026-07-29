//! Template matching and immutable result access.
//!
//! # Zero matches is a success
//!
//! A completed search that found nothing answers the question it was asked. It
//! returns `MADOPILOT_STATUS_OK` with a result whose count is zero, correlated
//! with the frame that was searched exactly as a result with matches would be.
//! Reporting it as a failure would make "is this on screen?" unanswerable in the
//! negative.
//!
//! # What a result handle keeps alive
//!
//! The envelope owns the exact frame it searched, so a result stays correlated
//! after the session, the template, the package, and the engine are gone. That
//! is what makes "which frame is this about" answerable at any later point, and
//! it is why releasing every other handle cannot invalidate a retained result.
//!
//! The effective options are read back out of the result rather than stored
//! beside it: `MatchResult::options` reports what the search ran under, so this
//! boundary keeps no second copy that could disagree with it.

use mado_pilot::{
    Error, FindOutcome, FindRequest, MatchOptions, PixelRect, PreparedTemplate, RegionSelection,
    Status,
};

use crate::boundary::{self, Out, Versioned, covers, declared, inputs, prefixes};
use crate::capture::{FrameHandle, SessionHandle, madopilot_session_t, rect, stamp};
use crate::engine::report;
use crate::error::{Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_VISION, MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK,
    madopilot_status_t,
};
use crate::types::{
    MADOPILOT_FIND_HAS_REGION, MADOPILOT_MATCH_HAS_MAX_RESULTS, MADOPILOT_MATCH_HAS_MIN_SCORE,
    MADOPILOT_MATCH_HAS_SUPPRESSION, clip_policy, madopilot_find_request_t,
    madopilot_frame_stamp_t, madopilot_match_options_t, madopilot_match_t, madopilot_operation_t,
    madopilot_pixel_rect_t, madopilot_result_info_t, madopilot_suppression_t, suppression,
    suppression_code,
};
use crate::view::madopilot_str_t;
use crate::{handle, hooks};

opaque! {
    /// One completed search, and everything it reports about.
    madopilot_result_t => ResultHandle
}

/// The payload behind a result handle.
#[derive(Debug)]
pub(crate) struct ResultHandle {
    outcome: FindOutcome,
    stream: u64,
}

inputs! {
    impl Input for madopilot_find_request_t {
        // Through `tmpl`: which frame and which template is the whole question.
        const MANDATORY: usize = 24;
        const NAME: &'static str = "madopilot_find_request_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_find_request_t,
            struct_size,
            flags,
            frame,
            tmpl,
            options,
            region,
            clip_policy,
        );
        const PRESENCE: &'static [(u32, usize)] = &[(
            MADOPILOT_FIND_HAS_REGION,
            covers!(madopilot_find_request_t, region: madopilot_pixel_rect_t),
        )];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                frame: std::ptr::null(),
                tmpl: std::ptr::null(),
                options: std::ptr::null(),
                region: madopilot_pixel_rect_t::empty(),
                clip_policy: crate::types::MADOPILOT_CLIP_POLICY_REJECT,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }

    impl Input for madopilot_match_options_t {
        // Through `flags`: an options structure that sets no bit is the documented
        // way of saying "the template's own defaults".
        const MANDATORY: usize = 8;
        const NAME: &'static str = "madopilot_match_options_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_match_options_t,
            struct_size,
            flags,
            min_score,
            max_results,
            suppression,
        );
        // The mandatory prefix stops at `flags`, so every one of these bits can name
        // a field the caller's own size leaves out. Honoring one there would put
        // `cleared`'s zero where the template's default belongs, and a `min_score`
        // of zero is the threshold that qualifies everything.
        const PRESENCE: &'static [(u32, usize)] = &[
            (
                MADOPILOT_MATCH_HAS_MIN_SCORE,
                covers!(madopilot_match_options_t, min_score: f64),
            ),
            (
                MADOPILOT_MATCH_HAS_MAX_RESULTS,
                covers!(madopilot_match_options_t, max_results: u32),
            ),
            (
                MADOPILOT_MATCH_HAS_SUPPRESSION,
                covers!(madopilot_match_options_t, suppression: madopilot_suppression_t),
            ),
        ];

        fn defaults() -> Self {
            Self::cleared(0)
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
}

impl Versioned for madopilot_match_options_t {
    const MANDATORY: usize = 24;
    const NAME: &'static str = "madopilot_match_options_t";

    fn failure(struct_size: u32) -> Self {
        Self::cleared(struct_size)
    }
}

impl Versioned for madopilot_match_t {
    // The whole structure: a match without a score and a place is not one, and
    // `bounds` is its last field.
    const MANDATORY: usize = 56;
    const NAME: &'static str = "madopilot_match_t";

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            score: 0.0,
            template_id: madopilot_str_t::empty(),
            bounds: madopilot_pixel_rect_t::empty(),
        }
    }
}

impl Versioned for madopilot_result_info_t {
    // The whole structure; `searched` is its last field.
    const MANDATORY: usize = 72;
    const NAME: &'static str = "madopilot_result_info_t";

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            match_count: 0,
            backend_id: madopilot_str_t::empty(),
            backend_version: madopilot_str_t::empty(),
            searched: madopilot_pixel_rect_t::empty(),
        }
    }
}

pub(crate) fn session_find(
    session: *const madopilot_session_t,
    request: *const madopilot_find_request_t,
    operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_result_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(fault) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_handle_and_error_out(out_result, "out_result", out_error) }
    {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_session_find(session, request, operation, out_result),
        )
    }
}

fn run_session_find(
    session: *const madopilot_session_t,
    request: *const madopilot_find_request_t,
    operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_result_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the session retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };

    // A closed session starts no further work. The Rust contract now makes the
    // same check for every frame choice — aligning the two was part of
    // `docs/adr/0006-public-rust-names-and-compatibility-policy.md` — and this
    // one stays because it refuses before any pointer below is dereferenced and
    // reports the boundary's own message and category.
    if session.session().is_closed() {
        return Err(Fault::closed(SESSION_CLOSED));
    }

    // SAFETY: the caller keeps the request structure readable for the call.
    let request = unsafe { boundary::read_input::<madopilot_find_request_t>(request) }?;
    // SAFETY: as above.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    // SAFETY: the caller keeps every handle the request names retained for the
    // call, which is the documented rule for a handle passed by pointer.
    let Some(template) = (unsafe { handle::borrow::<PreparedTemplate>(request.tmpl) }) else {
        return Err(Fault::abi("`tmpl` is null"));
    };
    // SAFETY: as above; null is the documented "use the session's latest frame".
    let frame = unsafe { handle::borrow::<FrameHandle>(request.frame) };

    let options = resolve_options(&request, template)?;
    let mut find = match frame {
        Some(frame) => FindRequest::exact(frame.frame(), template, options),
        None => FindRequest::latest(template, options),
    };

    if declared!(request, madopilot_find_request_t, MADOPILOT_FIND_HAS_REGION) {
        find = find.in_region(region_selection(&request)?);
    }

    let outcome = session
        .session()
        .find_template(&find, context.inner())
        .map_err(|error| search_failure(&error, session.backend()))?;
    hooks::reach(hooks::Site::AfterTemporary);

    // The search produced a perfectly good answer, and the operation may still
    // have run out of time producing it. A result that loses this race is
    // dropped rather than published.
    context.commit()?;

    let payload = ResultHandle {
        outcome,
        stream: frame.map_or_else(|| session.stream(), FrameHandle::stream),
    };
    // SAFETY: `out_result` was validated by the entry before any work began.
    unsafe { out_result.write(handle::into_raw(payload)) };

    Ok(())
}

/// What a session that has stopped accepting work reports, whichever side
/// observes it.
const SESSION_CLOSED: &str = "the session has closed and starts no further work";

/// Reports why a search produced no outcome.
///
/// One case needs naming. A session that has begun closing but not finished
/// draining refuses work and is not what `session_is_closed` calls closed, so
/// the fast path above lets it through and the search refuses it instead. The
/// outcome is the same `MADOPILOT_STATUS_CLOSED` either way, and the report says
/// so: a lifecycle refusal keeps the capture category and the boundary's own
/// message rather than arriving as a vision failure because of which side
/// happened to observe it. Every other failure is the search's own, and carries
/// the backend that ran it.
fn search_failure(error: &Error, backend: &str) -> Fault {
    if error.status() == Status::Closed {
        return Fault::closed(SESSION_CLOSED);
    }

    Fault::from_error(error, MADOPILOT_ERROR_CATEGORY_VISION).with_backend(backend)
}

fn resolve_options(
    request: &madopilot_find_request_t,
    template: &PreparedTemplate,
) -> Result<MatchOptions, Fault> {
    let mut options = MatchOptions::from_defaults(template.defaults());
    if request.options.is_null() {
        return Ok(options);
    }

    // SAFETY: the caller keeps the options structure readable for the call, and
    // `read_input` validates its alignment and declared size before reading.
    let supplied = unsafe { boundary::read_input::<madopilot_match_options_t>(request.options) }?;

    if declared!(
        supplied,
        madopilot_match_options_t,
        MADOPILOT_MATCH_HAS_MIN_SCORE
    ) {
        options = options
            .with_min_score(supplied.min_score)
            .map_err(|fault| Fault::from_error(&fault.into(), MADOPILOT_ERROR_CATEGORY_VISION))?;
    }
    if declared!(
        supplied,
        madopilot_match_options_t,
        MADOPILOT_MATCH_HAS_MAX_RESULTS
    ) {
        options = options
            .with_max_results(supplied.max_results)
            .map_err(|fault| Fault::from_error(&fault.into(), MADOPILOT_ERROR_CATEGORY_VISION))?;
    }
    if declared!(
        supplied,
        madopilot_match_options_t,
        MADOPILOT_MATCH_HAS_SUPPRESSION
    ) {
        options = options.with_suppression(suppression(supplied.suppression)?);
    }

    Ok(options)
}

fn region_selection(request: &madopilot_find_request_t) -> Result<RegionSelection, Fault> {
    let space = crate::types::space(request.region.space)?;
    if space != mado_pilot::CoordinateSpace::CapturePixels {
        return Err(Fault::abi(
            "Phase 1 accepts a search region in capture pixels only",
        ));
    }

    let region = PixelRect::new(
        request.region.left,
        request.region.top,
        request.region.right,
        request.region.bottom,
    )
    .map_err(|fault| {
        Fault::from_error(
            &fault.into(),
            crate::status::MADOPILOT_ERROR_CATEGORY_GEOMETRY,
        )
    })?;

    RegionSelection::pixels(region, clip_policy(request.clip_policy)?).map_err(|error| {
        Fault::from_error(&error, crate::status::MADOPILOT_ERROR_CATEGORY_GEOMETRY)
    })
}

pub(crate) fn result_retain(result: *const madopilot_result_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<ResultHandle>(result) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn result_release(result: *mut madopilot_result_t) -> madopilot_status_t {
    // SAFETY: as `result_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<ResultHandle>(result) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn result_describe(
    result: *const madopilot_result_t,
    out_info: *mut madopilot_result_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(result) = (unsafe { handle::borrow::<ResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let value = result.outcome.result();
    let count = u64::try_from(value.matches().len()).unwrap_or(u64::MAX);
    // SAFETY: `out` was validated above, and both views borrow from the result
    // the caller keeps retained.
    unsafe {
        out.commit(madopilot_result_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            match_count: count,
            backend_id: madopilot_str_t::borrowed(value.backend().id()),
            backend_version: madopilot_str_t::borrowed(value.backend().version()),
            searched: rect(value.searched()),
        });
    }

    MADOPILOT_STATUS_OK
}

pub(crate) fn result_stamp(
    result: *const madopilot_result_t,
    out_stamp: *mut madopilot_frame_stamp_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_stamp) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(result) = (unsafe { handle::borrow::<ResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(stamp(
            result.outcome.result().stamp(),
            result.stream,
            out.declared_size(),
        ));
    }

    MADOPILOT_STATUS_OK
}

pub(crate) fn result_options(
    result: *const madopilot_result_t,
    out_options: *mut madopilot_match_options_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::<madopilot_match_options_t>::begin(out_options) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(result) = (unsafe { handle::borrow::<ResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let effective = result.outcome.result().options();

    // Every field was in effect, so every presence bit is set: this reports what
    // the search actually ran under, not what the caller happened to ask for.
    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_match_options_t {
            struct_size: out.declared_size(),
            flags: MADOPILOT_MATCH_HAS_MIN_SCORE
                | MADOPILOT_MATCH_HAS_MAX_RESULTS
                | MADOPILOT_MATCH_HAS_SUPPRESSION,
            min_score: effective.min_score(),
            max_results: effective.max_results(),
            suppression: suppression_code(effective.suppression()),
        });
    }

    MADOPILOT_STATUS_OK
}

pub(crate) fn result_match(
    result: *const madopilot_result_t,
    index: usize,
    out_match: *mut madopilot_match_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_match) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(result) = (unsafe { handle::borrow::<ResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let matches = result.outcome.result().matches();
    match boundary::index_within(index, matches.len(), "match") {
        Ok(index) => {
            let found = &matches[index];
            // SAFETY: `out` was validated above, and the identity view borrows
            // from the result the caller keeps retained.
            unsafe {
                out.commit(madopilot_match_t {
                    struct_size: out.declared_size(),
                    flags: 0,
                    score: found.score(),
                    template_id: madopilot_str_t::borrowed(found.template().as_str()),
                    bounds: rect(found.bounds()),
                });
            }
            MADOPILOT_STATUS_OK
        }
        Err(fault) => fault.status(),
    }
}

#[cfg(test)]
mod tests {
    use crate::error::{self, madopilot_error_t};
    use crate::status::{
        MADOPILOT_ERROR_CATEGORY_CAPTURE, MADOPILOT_STATUS_CLOSED, MADOPILOT_STATUS_VISION_FAILED,
        madopilot_error_category_t,
    };
    use crate::types::madopilot_error_detail_t;
    use crate::{handle, view};

    use super::{
        Error, Fault, MADOPILOT_ERROR_CATEGORY_VISION, MADOPILOT_STATUS_OK, SESSION_CLOSED, Status,
        Versioned, madopilot_status_t, search_failure,
    };

    const BACKEND: &str = "test-backend";

    /// The status, category, and message a C caller reads back out of `fault`.
    ///
    /// Read through the boundary's own accessor rather than out of the fault's
    /// fields, because what this pins is what a caller can observe.
    fn reported(fault: Fault) -> (madopilot_status_t, madopilot_error_category_t, String) {
        let size = u32::try_from(size_of::<madopilot_error_detail_t>())
            .expect("a structure of a few dozen bytes");
        let mut detail = <madopilot_error_detail_t as Versioned>::failure(size);
        let handle: *mut madopilot_error_t = handle::into_raw(fault);

        assert_eq!(
            error::describe(handle, &raw mut detail),
            MADOPILOT_STATUS_OK
        );
        // SAFETY: the message view borrows from the fault behind `handle`, which
        // is still retained here, and `describe` wrote it from a `&str`.
        let message = unsafe { view::string(detail.message, "message") }
            .expect("the message the fault was built from")
            .to_owned();
        // The handle is the one produced above and this is its final release,
        // which is why the message is copied first.
        error::release(handle);

        (detail.status, detail.category, message)
    }

    /// A session that refuses work because it is closing reports what a closed
    /// session reports.
    ///
    /// The two are refused on different sides — the fast path in
    /// `run_session_find` catches only a session whose close has finished, and
    /// the search itself catches one that has merely begun — and a caller
    /// cannot tell which side it reached. Reporting the same outcome as a
    /// capture-lifecycle refusal one way and a vision failure the other would
    /// make the category describe the implementation rather than the failure.
    #[test]
    fn a_search_refused_because_the_session_is_closing_reports_the_lifecycle_refusal() {
        let closing = Error::new(Status::Closed, "the session is closing");

        assert_eq!(
            reported(search_failure(&closing, BACKEND)),
            reported(Fault::closed(SESSION_CLOSED))
        );
        assert_eq!(
            reported(search_failure(&closing, BACKEND)).0,
            MADOPILOT_STATUS_CLOSED
        );
        assert_eq!(
            reported(search_failure(&closing, BACKEND)).1,
            MADOPILOT_ERROR_CATEGORY_CAPTURE
        );
    }

    /// Every other search failure is still the search's own.
    #[test]
    fn a_search_that_could_not_run_reports_a_vision_failure() {
        let unavailable = Error::new(Status::VisionFailed, "the backend is unavailable");
        let (status, category, message) = reported(search_failure(&unavailable, BACKEND));

        assert_eq!(status, MADOPILOT_STATUS_VISION_FAILED);
        assert_eq!(category, MADOPILOT_ERROR_CATEGORY_VISION);
        assert_eq!(message, "the backend is unavailable");
    }
}
