//! The MadoPilot C ABI.
//!
//! # Responsibility
//!
//! This package owns the separately versioned C boundary: one exported
//! `extern "C"` entry point, an immutable function table reached through it,
//! opaque handles with a complete retain and release lifecycle, size-versioned
//! structures with documented mandatory prefixes, explicit pointer-length views,
//! module-owned allocation and release, and panic containment at every entry.
//!
//! It also owns the C header, direct C/C++ samples, boundary verification
//! consumers, and the development-tree `MadoPilot::C` CMake target.
//!
//! # Allowed seam
//!
//! This package depends on the `mado-pilot` facade only. The facade never
//! depends on it, and nothing here reaches past the facade into the runtime,
//! platform, or backend packages.
//!
//! # The one exported symbol
//!
//! [`madopilot_get_api`] is the only symbol this library exports. Every
//! operation is a member of the [`madopilot_api_t`] table it returns, so a
//! caller that negotiated a table has, by construction, negotiated everything it
//! can call. The table is a `'static` immutable object owned by the library and
//! is never released.
//!
//! # The Rust surface of this package
//!
//! The public Rust items below exist so that this package's own tests and its
//! `c-abi-check` example can measure what the C compiler is being asked to
//! agree with. They are not an intended Rust integration surface: a Rust host
//! uses the `mado-pilot` facade, which is what this package itself calls.
//!
//! # Implementation status
//!
//! ABI 1.5 preserves the complete released ABI 1.0, 1.2, 1.3, and 1.4
//! prefixes. ABI 1.0 covers deterministic capture/matching; ABI 1.2 appends
//! native input and bounded diagnostics; ABI 1.3 appends one-shot OCR; ABI 1.4
//! appends explicit profiles and grouped OCR; ABI 1.5 appends provider-policy
//! construction and immutable provider facts. It contains no watcher, query,
//! callback, automatic-input, or native-frame entry.
//!
//! C and C++ consume the same C ABI. Applications own handle management,
//! request storage, and higher-level policy; no official C++ wrapper is provided.
//!
//! **Every released numeric value, structure prefix, field offset, and table
//! position is frozen for ABI major 1.** Within this major nothing moves; a later
//! minor appends. `docs/c-abi.md` defines the compatibility contract.
//! `tests/abi-compat/` compiles and runs every
//! released header against later builds. The unreleased 1.1 draft has no fixture
//! or compatibility surface.
//!
//! # Where to start
//!
//! Start with `examples/c/direct-abi.c` or `examples/cpp/direct-abi.cpp`.
//! Both use `include/madopilot/madopilot.h` directly. `docs/c-abi.md` records
//! ownership, structure-prefix, status, and build rules. The larger C examples
//! are replay, native-input, and OCR verification consumers.

// The C surface is named the way C names it. Rust casing here would make the
// header and the definitions it mirrors two vocabularies for one contract.
#![allow(non_camel_case_types)]

// Panic containment is the whole of what `boundary::boundary` does, and
// `catch_unwind` catches nothing under an aborting profile: the process ends
// instead of the entry returning `MADOPILOT_STATUS_INTERNAL_PANIC`. That build
// produced a library whose documented behaviour was silently false, and it
// succeeded. It stops here rather than at whatever crashes first.
//
// This covers `-C panic=abort` and the `panic` profile key. It cannot cover
// `panic_immediate_abort`, which is a `std` feature selected through
// `-Z build-std` and is not visible to a dependent crate's `cfg`; `docs/c-abi.md`
// states that limit.
#[cfg(panic = "abort")]
compile_error!(
    "mado-pilot-capi cannot be built with an aborting panic profile. Every table \
     entry promises to contain a panic and return MADOPILOT_STATUS_INTERNAL_PANIC, \
     which requires unwinding; under `-C panic=abort` a contained panic ends the \
     host process instead. See docs/c-abi.md, \"Panic containment\"."
);

mod assets;
mod boundary;
mod capture;
mod diagnostic;
mod engine;
mod error;
#[cfg(feature = "private-fixture")]
mod fixture;
mod handle;
mod hooks;
mod input;
pub mod layout;
mod matching;
mod ocr;
mod operation;
mod status;
mod table;
mod types;
mod view;

pub use assets::{madopilot_package_t, madopilot_template_t};
pub use capture::{madopilot_frame_t, madopilot_mapping_t, madopilot_session_t};
pub use diagnostic::{madopilot_diagnostic_batch_t, madopilot_diagnostic_reader_t};
pub use engine::{madopilot_engine_t, madopilot_target_list_t};
pub use error::madopilot_error_t;
pub use input::madopilot_input_receipt_t;
pub use matching::madopilot_result_t;
pub use ocr::madopilot_ocr_result_t;
pub use operation::madopilot_cancellation_t;
pub use status::*;
pub use table::*;
pub use types::*;
pub use view::{madopilot_bytes_t, madopilot_str_t};
