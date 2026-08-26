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
//! It also holds the artifacts that are not Rust: the tracked C header, the
//! header-only C++ wrapper over it, the deterministic and native examples, the
//! layout, ownership, compatibility, and consumer probes, and the CMake project
//! that defines `MadoPilot::C` and `MadoPilot::Cpp`. None of those is a Cargo
//! target — Cargo ignores a directory under `examples/` or `tests/` with no
//! `main.rs` — and the C++ wrapper is not a Cargo package. They live here
//! because they are this boundary.
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
//! The C++ wrapper covers exactly the negotiated table and declares no ABI of
//! its own, so it adds no binary compatibility surface;
//! `docs/adr/0005-cpp-wrapper-shape-and-cmake-surface.md` records why it is
//! header-only.
//!
//! **Every released numeric value, structure prefix, field offset, and table
//! position is frozen for ABI major 1.** ADR 0007 froze ABI 1.0, ADR 0023 froze
//! ABI 1.2, ADR 0035 records ABI 1.3, ADR 0043 records ABI 1.4, and ADR 0046
//! records the additive ABI 1.5 provider boundary. Within this major nothing
//! moves; a later minor appends. `tests/abi-compat/` compiles and runs every
//! released header against later builds. The unreleased 1.1 draft has no fixture
//! or compatibility surface.
//!
//! # Where to start
//!
//! `include/madopilot/madopilot.h` is the contract as a C caller reads it.
//! `examples/c/deterministic-slice.c` and `examples/c/native-input-common.h`
//! are its replay and native common flows. The corresponding C++ surfaces are
//! `include/madopilot/madopilot.hpp`,
//! `examples/cpp/deterministic-slice.cpp`, and
//! `examples/cpp/native-input.cpp`. `docs/c-abi.md` records the ownership,
//! structure-prefix, status, and build rules, and `docs/cpp-wrapper.md` records
//! what the C++ adapter adds on top.

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
