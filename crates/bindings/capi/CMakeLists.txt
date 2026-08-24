# The MadoPilot C and C++ consumption surface.
#
# This project builds nothing of MadoPilot's own. `cargo` builds the library;
# this describes what a C or C++ consumer needs in order to use it, as the two
# targets `docs/architecture.md` reserves:
#
#   MadoPilot::C    the released C ABI — the header and the built library
#   MadoPilot::Cpp  the header-only C++ wrapper, which links transitively to it
#
# `MadoPilot::Cpp` is an INTERFACE target because the wrapper is header-only. It
# has no ABI of its own and no object code to produce; see
# `docs/adr/0005-cpp-wrapper-shape-and-cmake-surface.md`.
#
# Use it from a consumer project with:
#
#   add_subdirectory(<repo>/crates/bindings/capi madopilot)
#   target_link_libraries(my-app PRIVATE MadoPilot::Cpp)
#
# There is no install or export set yet. Release packaging is not implemented —
# the ABI-major decorated loader names are a packaging step this phase does not
# take — so consumption is from the development tree only. See `docs/c-abi.md`.

cmake_minimum_required(VERSION 3.22)

project(
  MadoPilot
  VERSION 0.3.0
  DESCRIPTION "Headless visual automation runtime — C ABI and C++ wrapper"
  LANGUAGES C CXX
)

# ---------------------------------------------------------------------------
# Where cargo put the library
#
# The path depends on the profile and on CARGO_TARGET_DIR, neither of which
# CMake can know. `c-abi-check` passes the directory it derived from its own
# executable; a human running cmake by hand gets the default below.
# ---------------------------------------------------------------------------

set(MADOPILOT_ARTIFACT_DIR
    "${CMAKE_CURRENT_LIST_DIR}/../../../target/debug"
    CACHE PATH
    "Directory holding the cargo-built MadoPilot library"
)

if(WIN32)
  set(_madopilot_library "${MADOPILOT_ARTIFACT_DIR}/madopilot.dll")
  # Cargo names a cdylib's import library `<name>.dll.lib`. The undecorated
  # `<name>.lib` is accepted too, so a later packaging change that renames it
  # does not have to touch this file.
  set(_madopilot_implib "${MADOPILOT_ARTIFACT_DIR}/madopilot.dll.lib")
  if(NOT EXISTS "${_madopilot_implib}")
    set(_madopilot_implib "${MADOPILOT_ARTIFACT_DIR}/madopilot.lib")
  endif()
elseif(APPLE)
  set(_madopilot_library "${MADOPILOT_ARTIFACT_DIR}/libmadopilot.dylib")
  set(_madopilot_implib "${_madopilot_library}")
else()
  set(_madopilot_library "${MADOPILOT_ARTIFACT_DIR}/libmadopilot.so")
  set(_madopilot_implib "${_madopilot_library}")
endif()

foreach(_madopilot_required IN ITEMS "${_madopilot_library}" "${_madopilot_implib}")
  if(NOT EXISTS "${_madopilot_required}")
    message(
      FATAL_ERROR
      "${_madopilot_required} does not exist.\n"
      "Build it first:\n"
      "    cargo build --locked --package mado-pilot-capi\n"
      "or point -DMADOPILOT_ARTIFACT_DIR=<dir> at the profile directory that has it."
    )
  endif()
endforeach()

# ---------------------------------------------------------------------------
# MadoPilot::C — the released C ABI
# ---------------------------------------------------------------------------

add_library(MadoPilot::C SHARED IMPORTED GLOBAL)

set_target_properties(
  MadoPilot::C
  PROPERTIES
    IMPORTED_LOCATION "${_madopilot_library}"
    IMPORTED_IMPLIB "${_madopilot_implib}"
    INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_CURRENT_LIST_DIR}/include"
)

if(NOT WIN32)
  # rustc gives a cdylib an `@rpath`-relative install name, so a consumer needs
  # a run path to the development artifact. A released, packaged library would
  # be installed somewhere the loader already searches; this is a property of
  # consuming the build tree, not of the ABI.
  #
  # A comma in the artifact directory cannot be expressed and is refused here
  # rather than at link time. `LINKER:` separates on commas, and so does the
  # `-Wl,` it expands to on both non-Windows release targets, so the run path
  # arrives split in two and its tail becomes a stray input file. Measured, not
  # assumed: an artifact directory ending `art,dir` fails with
  # `ld: file cannot be open()ed ... path=dir`, which names neither the option
  # nor the variable that produced it. Writing the option another way does not
  # help — the driver's own separator is the one that splits it.
  if(MADOPILOT_ARTIFACT_DIR MATCHES ",")
    message(
      FATAL_ERROR
      "MADOPILOT_ARTIFACT_DIR contains a comma: ${MADOPILOT_ARTIFACT_DIR}\n"
      "A run path reaches the linker through -Wl, which separates on commas, so \
such a path cannot be passed. Build into a directory without one."
    )
  endif()
  set_target_properties(
    MadoPilot::C
    PROPERTIES INTERFACE_LINK_OPTIONS "LINKER:-rpath,${MADOPILOT_ARTIFACT_DIR}"
  )
endif()

# ---------------------------------------------------------------------------
# MadoPilot::Cpp — the header-only wrapper
#
# The same include directory, C++17, and the C ABI underneath. Nothing else: an
# independent implementation of capture, mapping, matching, errors, or ABI
# negotiation is exactly what this target must not contain.
# ---------------------------------------------------------------------------

add_library(madopilot_cpp INTERFACE)
add_library(MadoPilot::Cpp ALIAS madopilot_cpp)

target_include_directories(madopilot_cpp INTERFACE "${CMAKE_CURRENT_LIST_DIR}/include")
target_compile_features(madopilot_cpp INTERFACE cxx_std_17)
target_link_libraries(madopilot_cpp INTERFACE MadoPilot::C)

# ---------------------------------------------------------------------------
# The examples are not built here
#
# This project ends at the two targets above, which is the surface ADR 0005
# declares. It carried a `MADOPILOT_BUILD_EXAMPLES` option and two executables
# behind it; nothing in the repository set the option, so the branch was never
# configured on either release target and would have rotted without anything
# reporting it. Nothing was lost by removing it: `c-abi-check` compiles, links,
# and runs both examples itself, with the release target's own compiler and
# flags, and building them a second time through CMake would have added no
# answer that check does not already give.
#
# A consumer that wants to build them adds its own `add_executable` against
# `MadoPilot::C` or `MadoPilot::Cpp`, which is what those targets are for.
# ---------------------------------------------------------------------------
