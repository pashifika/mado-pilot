# Independent consumer project driven by c-abi-check.
# Both languages link only MadoPilot::C; C_ONLY omits CXX entirely.

cmake_minimum_required(VERSION 3.22)

project(MadoPilotCMakeConsumer LANGUAGES C)

option(MADOPILOT_C_ONLY "Enable only C and omit the C++ consumer" OFF)
option(MADOPILOT_WINDOWS_PATH_ACCEPTANCE "Require controlled Windows OCR instead of skipping" OFF)
if(MADOPILOT_WINDOWS_PATH_ACCEPTANCE AND NOT WIN32)
  message(FATAL_ERROR "Windows path acceptance requires Windows")
endif()
if(NOT MADOPILOT_C_ONLY)
  enable_language(CXX)
endif()

if(NOT DEFINED MADOPILOT_SOURCE_DIR)
  message(
    FATAL_ERROR
    "Pass -DMADOPILOT_SOURCE_DIR=<repo>/crates/bindings/capi so this project can "
    "find the MadoPilot CMake target."
  )
endif()

# A single-config generator with no build type produces an unoptimized build
# with no diagnostics worth having. Multi-config generators ignore this and take
# `--config` instead.
if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
  set(CMAKE_BUILD_TYPE Release)
endif()

add_subdirectory("${MADOPILOT_SOURCE_DIR}" madopilot)

# --- The introductory samples: real handle lifecycle through the C ABI -------

add_executable(consumer-c "${MADOPILOT_SOURCE_DIR}/examples/c/direct-abi.c")
target_link_libraries(consumer-c PRIVATE MadoPilot::C)
set_target_properties(consumer-c PROPERTIES C_STANDARD 11 C_STANDARD_REQUIRED ON)
set(_madopilot_consumer_tests consumer-c)

if(NOT MADOPILOT_C_ONLY)
  add_executable(consumer-cpp "${MADOPILOT_SOURCE_DIR}/examples/cpp/direct-abi.cpp")
  target_link_libraries(consumer-cpp PRIVATE MadoPilot::C)
  set_target_properties(
    consumer-cpp
    PROPERTIES CXX_STANDARD 17 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF
  )
  list(APPEND _madopilot_consumer_tests consumer-cpp)
endif()

# --- The explicit bounded-profile grouped C example --------------------------
#
# It runs only when the controlled prerequisites are present in the environment
# and reports a CTest skip otherwise.

add_executable(
  consumer-ocr-profile-c
  "${MADOPILOT_SOURCE_DIR}/examples/c/ocr-profile-zones.c"
)
target_link_libraries(consumer-ocr-profile-c PRIVATE MadoPilot::C)
set_target_properties(
  consumer-ocr-profile-c
  PROPERTIES C_STANDARD 11 C_STANDARD_REQUIRED ON
)
list(APPEND _madopilot_consumer_tests consumer-ocr-profile-c)

if(WIN32)
  foreach(_sample RANGE 0 5)
    add_executable(windows-ingress-${_sample} windows-ingress.c)
    target_compile_definitions(windows-ingress-${_sample} PRIVATE TEST_SAMPLE=${_sample})
    target_include_directories(
      windows-ingress-${_sample} PRIVATE
      "${MADOPILOT_SOURCE_DIR}/include"
      "${MADOPILOT_SOURCE_DIR}/examples"
    )
    set_target_properties(
      windows-ingress-${_sample} PROPERTIES C_STANDARD 11 C_STANDARD_REQUIRED ON
    )
    list(APPEND _madopilot_consumer_tests windows-ingress-${_sample})
  endforeach()
endif()

enable_testing()

foreach(_madopilot_test IN LISTS _madopilot_consumer_tests)
  add_test(NAME ${_madopilot_test} COMMAND ${_madopilot_test})
endforeach()
set_tests_properties(${_madopilot_consumer_tests} PROPERTIES TIMEOUT 60)
if(MADOPILOT_WINDOWS_PATH_ACCEPTANCE)
  if(NOT IS_DIRECTORY "${MADOPILOT_ACCEPTANCE_MODEL_ROOT}" OR
     NOT EXISTS "${MADOPILOT_ACCEPTANCE_RUNTIME}")
    message(FATAL_ERROR "Windows path acceptance requires controlled model/runtime paths")
  endif()
  set_tests_properties(
    consumer-ocr-profile-c PROPERTIES ENVIRONMENT
    "MADO_PILOT_G004_MODEL_ROOT=${MADOPILOT_ACCEPTANCE_MODEL_ROOT};MADO_PILOT_ONNX_RUNTIME=${MADOPILOT_ACCEPTANCE_RUNTIME}"
  )
else()
  set_tests_properties(consumer-ocr-profile-c PROPERTIES SKIP_RETURN_CODE 77)
endif()

if(WIN32)
  # Windows has no run path: the loader searches the executable's directory and
  # then PATH. The tests get the library's directory prepended for their own
  # process only.
  file(TO_NATIVE_PATH "${MADOPILOT_ARTIFACT_DIR}" _madopilot_native_artifacts)
  set_tests_properties(
    ${_madopilot_consumer_tests}
    PROPERTIES
      ENVIRONMENT_MODIFICATION "PATH=path_list_prepend:${_madopilot_native_artifacts}"
  )
endif()
