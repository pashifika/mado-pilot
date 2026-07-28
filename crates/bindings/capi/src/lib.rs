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
//! header-only C++ wrapper over it, both examples, the layout and ownership
//! probes, and the CMake project that defines `MadoPilot::C` and
//! `MadoPilot::Cpp`. None of those is a Cargo target — cargo ignores a directory
//! under `examples/` or `tests/` with no `main.rs` — and the C++ wrapper is not
//! a Cargo package. They live here because they are this boundary.
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
//! Phase 1, complete. The table's Phase 1 prefix covers build and clock
//! information, cancellation, structured errors, engine construction over a
//! deterministic replay source, asset package loading, template preparation,
//! target discovery, capture-session lifecycle, latest-frame access, CPU
//! mapping, template matching, and immutable result access. It contains no
//! input, OCR, watcher, query, callback, or native-frame entry, and none of
//! those is reserved as a null table slot: a later phase appends them.
//!
//! The C++ wrapper covers exactly that prefix and declares no ABI of its own,
//! so it adds no compatibility surface;
//! `docs/adr/0005-cpp-wrapper-shape-and-cmake-surface.md` records why it is
//! header-only.
//!
//! **Every status value, structure layout, field offset, and table position
//! here is frozen for ABI major 1** by
//! `docs/adr/0007-phase-1-c-abi-freeze.md`, which resolved gate `G-010`. Within
//! this major nothing changes its number and nothing moves; a later minor
//! appends and raises `MADOPILOT_ABI_MINOR`. `tests/abi-compat/` keeps the
//! frozen header and compiles it against every later build of this library.
//!
//! # Where to start
//!
//! `include/madopilot/madopilot.h` is the contract as a C caller reads it, and
//! `examples/c/deterministic-slice.c` is the complete Phase 1 flow in C.
//! `include/madopilot/madopilot.hpp` and `examples/cpp/deterministic-slice.cpp`
//! are the same two in C++. `docs/c-abi.md` records the ownership,
//! structure-prefix, status, and build rules all of those depend on, and
//! `docs/cpp-wrapper.md` records what the C++ adapter adds on top.

// The C surface is named the way C names it. Rust casing here would make the
// header and the definitions it mirrors two vocabularies for one contract.
#![allow(non_camel_case_types)]

mod assets;
mod boundary;
mod capture;
mod engine;
mod error;
mod handle;
mod hooks;
pub mod layout;
mod matching;
mod operation;
mod status;
mod table;
mod types;
mod view;

pub use assets::{madopilot_package_t, madopilot_template_t};
pub use capture::{madopilot_frame_t, madopilot_mapping_t, madopilot_session_t};
pub use engine::{madopilot_engine_t, madopilot_target_list_t};
pub use error::madopilot_error_t;
pub use matching::madopilot_result_t;
pub use operation::madopilot_cancellation_t;
pub use status::*;
pub use table::*;
pub use types::*;
pub use view::{madopilot_bytes_t, madopilot_str_t};
