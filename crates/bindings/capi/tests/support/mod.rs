//! Shared setup for the C ABI tests.
//!
//! Everything here goes through the negotiated function table rather than
//! through the crate's Rust items, so the tests exercise the same entries a C
//! caller reaches. The Rust items are used only to name the structures those
//! entries read and write, which is what a C header does too.

// The module is shared by `mod support;` in each test binary, so items
// unused by one of them, and `pub` items no other crate can reach, are
// expected rather than accidental.
#![allow(dead_code, non_camel_case_types, unreachable_pub)]

use std::ffi::c_char;
use std::path::PathBuf;
use std::ptr;

use mado_pilot::PixelFormat;
use mado_pilot_testkit::match_fixtures;
use madopilot::layout::struct_size;
use madopilot::*;

/// Negotiates the complete Phase 1 table, as a C caller would.
pub fn table() -> &'static madopilot_api_t {
    negotiate(
        MADOPILOT_ABI_MAJOR,
        MADOPILOT_ABI_MINOR,
        size_of::<madopilot_api_t>(),
    )
    .expect("the current header negotiates the current library")
}

/// Negotiates with explicit parameters, returning the status on failure.
pub fn negotiate(
    abi_major: u32,
    min_abi_minor: u32,
    caller_struct_size: usize,
) -> Result<&'static madopilot_api_t, madopilot_status_t> {
    let mut api: *const madopilot_api_t = ptr::null();
    // SAFETY: `api` is a live, writable, correctly aligned local.
    let status =
        unsafe { madopilot_get_api(abi_major, min_abi_minor, caller_struct_size, &raw mut api) };

    if status == MADOPILOT_STATUS_OK {
        // SAFETY: negotiation succeeded, so `api` names the library's static
        // table, which lives as long as the library does.
        Ok(unsafe { api.as_ref() }.expect("a negotiated table is never null"))
    } else {
        assert!(api.is_null(), "a refused negotiation nulls its output");
        Err(status)
    }
}

/// An operation with no deadline and no cancellation.
pub fn operation() -> madopilot_operation_t {
    madopilot_operation_t {
        struct_size: struct_size::<madopilot_operation_t>(),
        flags: 0,
        deadline_nanos: 0,
        cancellation: ptr::null(),
    }
}

/// An operation whose deadline has already passed.
pub fn expired_operation() -> madopilot_operation_t {
    madopilot_operation_t {
        struct_size: struct_size::<madopilot_operation_t>(),
        // The domain origin is the earliest instant there is, so a deadline of
        // zero is expired from the first nanosecond of the process.
        flags: MADOPILOT_OPERATION_HAS_DEADLINE,
        deadline_nanos: 0,
        cancellation: ptr::null(),
    }
}

/// Borrows a Rust string as a C view.
pub fn str_view(value: &str) -> madopilot_str_t {
    madopilot_str_t {
        data: value.as_ptr().cast::<c_char>(),
        len: value.len(),
    }
}

/// Borrows a Rust slice as a C view.
pub fn bytes_view(value: &[u8]) -> madopilot_bytes_t {
    madopilot_bytes_t {
        data: value.as_ptr(),
        len: value.len(),
    }
}

/// The tracked two-template package the Rust example also loads.
pub fn package_root() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/assets/phase1-slice")
        .to_string_lossy()
        .into_owned()
}

/// The deterministic scene, and the structures that describe it.
///
/// Held together in one value because the source points at the frame and the
/// frame points at the pixels: separating them would leave a caller holding a
/// structure whose pointees had already been dropped.
pub struct Scene {
    pixels: Vec<u8>,
    frame: madopilot_replay_frame_t,
    source: madopilot_source_t,
}

impl Scene {
    pub fn new() -> Box<Self> {
        let pixels = match_fixtures::scene_pixels(PixelFormat::Rgba8);
        let mut scene = Box::new(Self {
            pixels,
            frame: madopilot_replay_frame_t {
                struct_size: struct_size::<madopilot_replay_frame_t>(),
                flags: 0,
                width: match_fixtures::SCENE.width(),
                height: match_fixtures::SCENE.height(),
                format: MADOPILOT_PIXEL_FORMAT_RGBA8,
                continuity: MADOPILOT_CONTINUITY_CONTINUOUS,
                pixels: madopilot_bytes_t::empty(),
                captured_at_nanos: 0,
                stride: 0,
            },
            source: madopilot_source_t {
                struct_size: struct_size::<madopilot_source_t>(),
                kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
                directory: madopilot_str_t::empty(),
                frames: ptr::null(),
                frame_count: 1,
                frame_stride: size_of::<madopilot_replay_frame_t>(),
                target_name: madopilot_str_t::empty(),
            },
        });

        scene.frame.pixels = bytes_view(&scene.pixels);
        scene.source.frames = &raw const scene.frame;

        scene
    }

    pub const fn source(&self) -> *const madopilot_source_t {
        &raw const self.source
    }

    /// The frame structure, for a test that wants to vary one field.
    pub const fn frame_input(&self) -> madopilot_replay_frame_t {
        self.frame
    }

    /// The source structure, for a test that wants to vary one field.
    ///
    /// The copy keeps pointing at this scene's frame, so it is usable only
    /// while the scene is alive — the same rule the pointer version follows.
    pub const fn source_input(&self) -> madopilot_source_t {
        self.source
    }
}

/// The whole Phase 1 chain, opened once and released in reverse on drop.
pub struct Flow {
    pub api: &'static madopilot_api_t,
    pub engine: *mut madopilot_engine_t,
    pub targets: *mut madopilot_target_list_t,
    pub session: *mut madopilot_session_t,
    pub frame: *mut madopilot_frame_t,
    pub package: *mut madopilot_package_t,
    pub present: *mut madopilot_template_t,
    pub absent: *mut madopilot_template_t,
    scene: Box<Scene>,
    root: String,
}

impl Flow {
    pub fn open() -> Self {
        let api = table();
        let scene = Scene::new();
        let operation = operation();
        let root = package_root();

        let mut engine = ptr::null_mut();
        // SAFETY: every pointer is a live local or an owned box that outlives
        // the call.
        let status = unsafe {
            (api.engine_create)(
                scene.source(),
                &raw const operation,
                &raw mut engine,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "engine_create");

        let mut targets = ptr::null_mut();
        // SAFETY: as above; the engine is retained by this value.
        let status = unsafe {
            (api.engine_discover)(
                engine,
                &raw const operation,
                &raw mut targets,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "engine_discover");

        let open = madopilot_open_request_t {
            struct_size: struct_size::<madopilot_open_request_t>(),
            flags: 0,
            required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        };
        let mut session = ptr::null_mut();
        // SAFETY: as above.
        let status = unsafe {
            (api.session_open)(
                engine,
                targets,
                0,
                &raw const open,
                &raw const operation,
                &raw mut session,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "session_open");

        let mut frame = ptr::null_mut();
        // SAFETY: as above.
        let status = unsafe {
            (api.session_acquire_frame)(
                session,
                &raw const operation,
                &raw mut frame,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "session_acquire_frame");

        let package_source = madopilot_package_source_t {
            struct_size: struct_size::<madopilot_package_source_t>(),
            kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
            path: str_view(&root),
            archive: madopilot_bytes_t::empty(),
        };
        let mut package = ptr::null_mut();
        // SAFETY: as above; `root` outlives the call.
        let status = unsafe {
            (api.package_load)(
                engine,
                &raw const package_source,
                &raw const operation,
                &raw mut package,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "package_load");

        let present = prepare(api, engine, package, "panel.patch");
        let absent = prepare(api, engine, package, "panel.absent");

        Self {
            api,
            engine,
            targets,
            session,
            frame,
            package,
            present,
            absent,
            scene,
            root,
        }
    }

    /// A find request against the held frame and the present template.
    pub fn find_request(&self) -> madopilot_find_request_t {
        madopilot_find_request_t {
            struct_size: struct_size::<madopilot_find_request_t>(),
            flags: 0,
            frame: self.frame,
            tmpl: self.present,
            options: ptr::null(),
            region: madopilot_pixel_rect_t::empty(),
            clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        }
    }

    /// Runs one search and returns the owned result.
    pub fn find(&self, request: &madopilot_find_request_t) -> *mut madopilot_result_t {
        let operation = operation();
        let mut result = ptr::null_mut();
        // SAFETY: every handle the request names is retained by this value, and
        // every pointer is a live local.
        let status = unsafe {
            (self.api.session_find)(
                self.session,
                &raw const *request,
                &raw const operation,
                &raw mut result,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "session_find");
        assert!(!result.is_null());

        result
    }

    /// Maps the whole held frame and returns the owned mapping.
    pub fn map(&self) -> *mut madopilot_mapping_t {
        let operation = operation();
        let request = madopilot_map_request_t {
            struct_size: struct_size::<madopilot_map_request_t>(),
            flags: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
            region: madopilot_pixel_rect_t::empty(),
        };
        let mut mapping = ptr::null_mut();
        // SAFETY: the frame is retained by this value, and every pointer is a
        // live local.
        let status = unsafe {
            (self.api.frame_map)(
                self.frame,
                &raw const request,
                &raw const operation,
                &raw mut mapping,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "frame_map");
        assert!(!mapping.is_null());

        mapping
    }

    /// Keeps the scene and the package path alive for as long as the flow.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn root(&self) -> &str {
        &self.root
    }
}

impl Drop for Flow {
    fn drop(&mut self) {
        // SAFETY: each handle was produced by this table and is owned here; the
        // release entries accept null, so a partially built flow is safe too.
        unsafe {
            (self.api.template_release)(self.absent);
            (self.api.template_release)(self.present);
            (self.api.package_release)(self.package);
            (self.api.frame_release)(self.frame);
            (self.api.session_release)(self.session);
            (self.api.target_list_release)(self.targets);
            (self.api.engine_release)(self.engine);
        }
    }
}

fn prepare(
    api: &'static madopilot_api_t,
    engine: *mut madopilot_engine_t,
    package: *mut madopilot_package_t,
    id: &str,
) -> *mut madopilot_template_t {
    let operation = operation();
    let mut prepared = ptr::null_mut();
    // SAFETY: both handles are retained by the caller, and `id` outlives the
    // call.
    let status = unsafe {
        (api.template_prepare_from_package)(
            engine,
            package,
            str_view(id),
            &raw const operation,
            &raw mut prepared,
            ptr::null_mut(),
        )
    };
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "template_prepare_from_package({id})"
    );

    prepared
}

/// Produces an owned error handle from a genuine refusal.
///
/// A test that needs an error to exercise the error entries themselves gets one
/// the library actually built, rather than a handle conjured some other way.
pub fn refused_error(api: &'static madopilot_api_t) -> *mut madopilot_error_t {
    let scene = Scene::new();
    let mut undersized = operation();
    undersized.struct_size = 0;

    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: every pointer is a live local, and the undersized operation is
    // refused after the outputs are initialized, which is what publishes the
    // owned error handle.
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const undersized,
            &raw mut engine,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(engine.is_null());
    assert!(
        !error.is_null(),
        "a refusal with an accepted out_error reports one"
    );

    error
}

/// Copies a C view's bytes as a Rust string.
///
/// The view borrows from whatever produced it, so anything a test keeps past
/// that owner's release has to be copied out first.
///
/// # Panics
///
/// Panics when a non-empty view is null, or when its bytes are not UTF-8.
pub fn view_to_string(view: madopilot_str_t) -> String {
    if view.len == 0 {
        return String::new();
    }
    assert!(!view.data.is_null(), "a non-empty view has a pointer");

    // SAFETY: the view is non-null with a length its producer set, and the
    // caller keeps its owner retained across this call.
    let bytes = unsafe { std::slice::from_raw_parts(view.data.cast::<u8>(), view.len) };
    String::from_utf8(bytes.to_vec()).expect("a message view is UTF-8")
}

/// Reads an error's structured detail and its message text, then releases it.
///
/// [`describe_and_release`] blanks the borrowed views, which is correct for a
/// caller keeping the detail but meant no Rust test ever read a message: an
/// error that reported an empty one passed `cargo test` and was caught only by
/// the C++ probe. This copies the text out first, so the message surface is
/// asserted in the same suite that asserts everything else.
pub fn describe_message_and_release(
    api: &'static madopilot_api_t,
    error: *mut madopilot_error_t,
) -> (madopilot_error_detail_t, String) {
    assert!(!error.is_null(), "a reported failure produced an error");

    let mut detail = madopilot_error_detail_t {
        struct_size: struct_size::<madopilot_error_detail_t>(),
        flags: 0,
        status: MADOPILOT_STATUS_OK,
        category: MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
        asset_fault: MADOPILOT_ASSET_FAULT_UNKNOWN,
        asset_stage: MADOPILOT_ASSET_STAGE_UNKNOWN,
        message: madopilot_str_t::empty(),
        backend: madopilot_str_t::empty(),
    };
    // SAFETY: `detail` is a live local with its `struct_size` set, and the
    // error is retained until the release below.
    let status = unsafe { (api.error_describe)(error, &raw mut detail) };
    assert_eq!(status, MADOPILOT_STATUS_OK, "error_describe");

    let message = view_to_string(detail.message);
    let copied = madopilot_error_detail_t {
        message: madopilot_str_t::empty(),
        backend: madopilot_str_t::empty(),
        ..detail
    };
    // SAFETY: the caller owns this reference and is giving it up.
    unsafe { (api.error_release)(error) };

    (copied, message)
}

/// Reads an error's structured detail, then releases it.
pub fn describe_and_release(
    api: &'static madopilot_api_t,
    error: *mut madopilot_error_t,
) -> madopilot_error_detail_t {
    assert!(!error.is_null(), "a reported failure produced an error");

    let mut detail = madopilot_error_detail_t {
        struct_size: struct_size::<madopilot_error_detail_t>(),
        flags: 0,
        status: MADOPILOT_STATUS_OK,
        category: MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
        asset_fault: MADOPILOT_ASSET_FAULT_UNKNOWN,
        asset_stage: MADOPILOT_ASSET_STAGE_UNKNOWN,
        message: madopilot_str_t::empty(),
        backend: madopilot_str_t::empty(),
    };
    // SAFETY: `detail` is a live local with its `struct_size` set, and the
    // error is retained until the release below.
    let status = unsafe { (api.error_describe)(error, &raw mut detail) };
    assert_eq!(status, MADOPILOT_STATUS_OK, "error_describe");

    let copied = madopilot_error_detail_t {
        // The views borrow from the error and die with it, so a caller that
        // keeps the detail past the release keeps only the numbers.
        message: madopilot_str_t::empty(),
        backend: madopilot_str_t::empty(),
        ..detail
    };
    // SAFETY: the caller owns this reference and is giving it up.
    unsafe { (api.error_release)(error) };

    copied
}
