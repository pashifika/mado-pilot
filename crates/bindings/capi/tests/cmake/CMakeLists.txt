# A consumer of the MadoPilot CMake targets, built as its own project.
#
# The point is that it is separate. It has its own cache, its own compiler
# flags, and knows nothing about MadoPilot except the two target names — so if
# `MadoPilot::Cpp` failed to carry the include directory, or failed to bring
# `MadoPilot::C` with it, this project would not build.
#
# Driven by `cargo run --package mado-pilot-capi --example c-abi-check`:
#
#   cmake -S tests/cmake -B <build> \
#         -DMADOPILOT_SOURCE_DIR=<repo>/crates/bindings/capi \
#         -DMADOPILOT_ARTIFACT_DIR=<repo>/target/debug
#   cmake --build <build> --config Release
#   ctest --test-dir <build> --build-config Release --output-on-failure

cmake_minimum_required(VERSION 3.22)

project(MadoPilotCMakeConsumer LANGUAGES C CXX)

if(NOT DEFINED MADOPILOT_SOURCE_DIR)
  message(
    FATAL_ERROR
    "Pass -DMADOPILOT_SOURCE_DIR=<repo>/crates/bindings/capi so this project can "
    "find the MadoPilot CMake targets."
  )
endif()

# A single-config generator with no build type produces an unoptimized build
# with no diagnostics worth having. Multi-config generators ignore this and take
# `--config` instead.
if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
  set(CMAKE_BUILD_TYPE Release)
endif()

add_subdirectory("${MADOPILOT_SOURCE_DIR}" madopilot)

# --- The C consumer: the released ABI on its own -----------------------------

add_executable(consumer-c consumer.c)
target_link_libraries(consumer-c PRIVATE MadoPilot::C)
set_target_properties(consumer-c PROPERTIES C_STANDARD 11 C_STANDARD_REQUIRED ON)

# --- The C++ consumer: the wrapper, and the C ABI through it -----------------
#
# It links `MadoPilot::Cpp` and nothing else. The include directory for
# `madopilot.hpp`, the C header it includes, and the library both resolve
# against arrive transitively or this does not compile and link.

add_executable(consumer-cpp consumer.cpp)
target_link_libraries(consumer-cpp PRIVATE MadoPilot::Cpp)

enable_testing()

add_test(NAME consumer-c COMMAND consumer-c)
add_test(NAME consumer-cpp COMMAND consumer-cpp)

if(WIN32)
  # Windows has no run path: the loader searches the executable's directory and
  # then PATH. The tests get the library's directory prepended for their own
  # process only.
  file(TO_NATIVE_PATH "${MADOPILOT_ARTIFACT_DIR}" _madopilot_native_artifacts)
  set_tests_properties(
    consumer-c consumer-cpp
    PROPERTIES
      ENVIRONMENT_MODIFICATION "PATH=path_list_prepend:${_madopilot_native_artifacts}"
  )
endif()
