# The C ABI

MadoPilot's C boundary is a separately versioned contract with its own
compatibility rules, declared in
[`crates/bindings/capi/include/madopilot/madopilot.h`](../crates/bindings/capi/include/madopilot/madopilot.h).
This guide covers handle lifetimes, `struct_size` prefix negotiation, status and
error semantics, OCR and input ownership, panic containment, and building C and
C++ consumers on `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`.

Both C and C++ callers include `madopilot/madopilot.h` and link the shared
`MadoPilot::C` CMake target directly. No official C++ wrapper or compatibility
shim is provided, so one ABI carries one ownership model and one compatibility
policy. Reference samples under
[`crates/bindings/capi/examples/`](../crates/bindings/capi/examples/) include:

- [`c/direct-abi.c`](../crates/bindings/capi/examples/c/direct-abi.c) and
  [`cpp/direct-abi.cpp`](../crates/bindings/capi/examples/cpp/direct-abi.cpp):
  minimal negotiation, build info, cancellation, and handle release.
- [`c/deterministic-slice.c`](../crates/bindings/capi/examples/c/deterministic-slice.c):
  replay capture and template matching.
- [`c/ocr-default.c`](../crates/bindings/capi/examples/c/ocr-default.c),
  [`c/ocr-profile-zones.c`](../crates/bindings/capi/examples/c/ocr-profile-zones.c),
  and [`c/ocr-fixture.c`](../crates/bindings/capi/examples/c/ocr-fixture.c):
  default, explicit-profile grouped-zone, and simulated OCR workflows.
- [`c/windows-native-input.c`](../crates/bindings/capi/examples/c/windows-native-input.c)
  and [`c/macos-native-input.c`](../crates/bindings/capi/examples/c/macos-native-input.c):
  native discovery, capture, and input verification consumers.

## Direct C and C++ consumption

Start with [`direct-abi.c`](../crates/bindings/capi/examples/c/direct-abi.c) or
[`direct-abi.cpp`](../crates/bindings/capi/examples/cpp/direct-abi.cpp), which
require only the built library (no models, permissions, or desktop targets).
Applications own handle management, error handling, request backing storage,
deadlines, cancellation, and sequencing: keep pointer-length inputs valid for
the duration of each borrowing call, read borrowed views only while their owner
handle remains retained, and release every owned handle explicitly.

On Windows, argument-consuming C samples convert UTF-16 `wmain` arguments (and
`_wgetenv` fallbacks in the profile OCR sample, with explicit arguments taking
precedence) into owned UTF-8 buffers before calling the ABI. Invalid UTF-16 or
allocation failure exits with an error rather than performing lossy replacement
or reporting a prerequisite skip. See
[Windows input verification](windows-input-verification.md#audit-2-6-and-16-acceptance)
for the non-ASCII C consumer acceptance contract.

## Released prefixes and compatibility

The current header declares **ABI 1.5**. Every released minor within ABI major 1
is a frozen complete prefix of the function table:

| ABI | Table extent | Appended surface |
|---|---|---|
| `1.0` | `424` bytes | Capture, mapping, assets, and template matching: 13 status values, 40-byte mandatory table prefix ending at `status_text`, and 23 fixed-layout structures ([layout reports](evidence/c-abi/)) |
| `1.2` | `592` bytes | Native route capability and submission-evidence vocabulary, owned input receipts and attempt accessors, operation activity tags, and bounded diagnostics (21 entries) |
| `1.3` | `648` bytes | Six-entry singular OCR suffix with immutable result/view ownership through offset `640`, plus `engine_create_with_default_ocr` at offset `640` |
| `1.4` | `720` bytes | `engine_create_with_ocr_profile`, `session_scan_ocr_zones`, and grouped-result owner/accessors at offsets `648–704`, plus `engine_ocr_descriptor` at offset `712` |
| `1.5` | `736` bytes | `engine_create_with_ocr_provider` at offset `720` and `engine_ocr_provider_descriptor` at offset `728`, with 32-byte provider options and a 40-byte engine-owned descriptor |

Within ABI major 1:

- No released enum/status numeric value, structure field offset, or function-table
  slot ever changes or is removed (`madopilot_engine_capabilities_t` stays 8
  bytes, `madopilot_engine_options_t` stays 16 bytes, and
  `madopilot_diagnostic_record_t` preserves its 240-byte ABI 1.2 prefix before
  its appended ABI 1.3 OCR fields).
- Later minor versions append only at the end of extensible structures or the
  function table and increment `MADOPILOT_ABI_MINOR`.
- The unreleased `1.1` development draft is unsupported and rejected (`minor == 1`).
- Product version `0.5.0` widened `session_scan_ocr_zones` from 8 to 16 zones
  without changing the ABI 1.5 table or structure layouts; watchers remain
  Rust-only.
- Callers negotiate with `madopilot_get_api`, use the smaller of
  `sizeof(madopilot_api_t)` and the returned `api->struct_size`, and verify both
  extents before reading any entry (e.g., `728` bytes for
  `MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_OCR_PROVIDER` and `736` bytes for
  `MADOPILOT_API_SIZE_ENGINE_OCR_PROVIDER_DESCRIPTOR`).

## Migrating an ABI 1.4 caller to provider-policy OCR

Existing ABI 1.4 constructors remain CPU-only. To configure an explicit OCR
execution provider in ABI 1.5:

1. Negotiate minor `5` and require `MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_OCR_PROVIDER` (`728` bytes).
2. Populate `madopilot_ocr_profile_options_t` with `MADOPILOT_OCR_PROFILE_G004` or
   `MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR` and canonical model-root/runtime paths.
3. Populate the 32-byte `madopilot_ocr_provider_options_t` with a closed provider
   policy (`CUDA` and `AUTO_PREFER_ACCELERATOR` may supply a canonical provider
   dependency root; `CPU` and `COREML` require an empty root).
4. Call `engine_create_with_ocr_provider` (`Prefer*` policies may fall back to a
   fresh CPU session pair only if accelerator initialization fails before engine
   publication; `Require*` policies return a typed error).
5. Require `MADOPILOT_API_SIZE_ENGINE_OCR_PROVIDER_DESCRIPTOR` (`736` bytes) and
   call `engine_ocr_provider_descriptor` to inspect the 40-byte engine-owned
   descriptor (requested policy, active provider, fallback status/reason, and
   borrowed runtime-profile string).

## Migrating from the unreleased 1.1 draft

ABI `1.1` was development-only and is rejected at negotiation. Recompile against
ABI `1.2` or newer using explicit input routes, owned receipts, submission
evidence, and bounded diagnostics.

## Migrating an ABI 1.2 caller to one-shot OCR

1. Compile against ABI `1.3`+ and require `MADOPILOT_API_SIZE_OCR_RESULT_TEXT_AT`
   (plus `MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_DEFAULT_OCR` when using
   `engine_create_with_default_ocr` and `madopilot_default_ocr_options_t`).
2. Retain the exact source frame and pass matching backend/model identities in
   `madopilot_ocr_request_t` (`package` may be null only when matching the engine's
   integrated profile reported by `engine_ocr_descriptor`).
3. Own the returned `madopilot_ocr_result_t`, read metadata and regions via
   `ocr_result_info`, `ocr_result_region_at`, and `ocr_result_text_at` (whose
   string views borrow from the result handle), and release with
   `ocr_result_release`.

## Migrating an ABI 1.3 caller to explicit profile and grouped OCR

1. Negotiate minor `4`+, construct with `engine_create_with_ocr_profile`
   (`MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR`), and read the configured identity
   via `engine_ocr_descriptor`.
2. Pass `1..=16` `madopilot_ocr_zone_t` elements (pointer, `size_t` count, and
   aligned `element_stride` in capture pixels) to `session_scan_ocr_zones`. Note
   that 9–16 zones require library version `0.5.0+` (`build_info.library_version`);
   older libraries accept `1..=8` zones.
3. Own the returned `madopilot_ocr_zone_scan_result_t` and inspect groups via
   `ocr_zone_scan_result_info`, `zone_at`, `region_at`, and `text_at` before
   calling `ocr_zone_scan_result_release`.

## One exported symbol

In release builds, `madopilot_get_api` is the sole exported symbol:

```c
const madopilot_api_t* api = NULL;
madopilot_status_t status = madopilot_get_api(
    MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, sizeof(madopilot_api_t), &api);
```

It returns `MADOPILOT_STATUS_UNSUPPORTED` for a different ABI major or a minimum
minor newer than the library, and `MADOPILOT_STATUS_INVALID_ARGUMENT` when
`caller_table_size < MADOPILOT_API_SIZE_INFORMATION` (`40` bytes) or `out_api` is
null.

## Handles

Every owned handle is reference counted:

| Rule | Contract |
|---|---|
| `*_retain` / `*_release` | Add or drop one reference; null is a no-op; the final release frees state with the module's allocator |
| Other table entries | Reject null handle arguments with `MADOPILOT_STATUS_INVALID_ARGUMENT` |
| Call lifetime & concurrency | Callers must hold a live reference for the entire duration of any call using the handle; concurrent `const` reads are thread-safe when each thread holds a live reference |
| Parent/child independence | Releasing a parent handle never invalidates a separately retained child (`mapping`, `prepared_template`, `match_result`, `ocr_result`, `ocr_zone_scan_result`, `input_receipt`, `diagnostic_reader`, or `diagnostic_batch`) |
| Borrowed views | Library-returned views (`madopilot_str_t`, `madopilot_bytes_t`) remain valid only while their documented owner handle is retained; caller-supplied input views are borrowed only for the duration of the synchronous call |

## Size-versioned structures

Every extensible structure starts with `uint32_t struct_size` followed by a
32-bit `flags` or discriminant field (avoiding implicit padding). Callers set
`struct_size = sizeof(...)` as declared by their header:

- **Input validation:** The boundary rejects `struct_size` values smaller than the
  mandatory prefix, ending mid-field, exceeding an array's `element_stride`, or
  omitting a field whose presence flag is set (`MADOPILOT_STATUS_INVALID_ARGUMENT`
  with `MADOPILOT_ERROR_CATEGORY_ABI`). Omitted optional tail fields receive
  documented defaults, and unknown trailing bytes are ignored.
- **Output writing:** The boundary validates the output `struct_size`, writes only
  within the caller's buffer, and updates `struct_size` to the number of bytes
  populated.
- **Dual-direction `madopilot_match_options_t`:** As an input, its mandatory
  prefix is `8` bytes (through `flags`); as the output of `result_options`, its
  mandatory prefix is the full `sizeof(madopilot_match_options_t)`.
- **Conditional input prefixes:** `madopilot_input_event_t` requires the prefix
  for its event kind (`button`, `key_value`, `x`/`y`, `horizontal`/`vertical`,
  `text`, or `delay_nanos`); `madopilot_input_request_t` requires
  `cleanup_timeout_nanos` when `MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET` is
  set; and `madopilot_ocr_request_t` requires `region` when
  `MADOPILOT_OCR_HAS_REGION` is set. Arrays of versioned structs pass explicit
  `frame_stride`, `event_stride`, or `zone_stride`.

## Validation, and what an output looks like on failure

Before validating request inputs, every valid output parameter is initialized to
its failure state (owned handles to `NULL`, structs through their failure prefix,
scalars to `0`). All pointer-length pairs, alignments, strides, and integer
ranges are checked before dereferencing. Every pointer parameter is required
unless documented as optional (`out_error`, `madopilot_operation_t.cancellation`,
`madopilot_find_request_t.{frame,options}`, `madopilot_ocr_request_t.package` for
matching integrated OCR profiles, `madopilot_input_request_t.source_frame` when
geometry policy does not need a frame snapshot, and zero-count array pointers).

## Deadlines, cancellation, and activity correlation

Every blocking entry requires a non-null `const madopilot_operation_t*`.
Deadlines are absolute instants in nanoseconds in the library's monotonic clock
domain (`clock_now`). `MADOPILOT_OPERATION_HAS_ACTIVITY_TAG` attaches a non-zero
opaque `activity_tag` to diagnostic records (and to the Core Graphics event
source user-data field on macOS `ProcessDirected` input) for observational
correlation without affecting admission or execution.

## Statuses, owned errors, and admitted receipts

Callers branch on `madopilot_status_t` (never on diagnostic text):

| Status | Meaning |
|---|---|
| `MADOPILOT_STATUS_OK` | Operation succeeded and populated required outputs |
| `MADOPILOT_STATUS_INVALID_ARGUMENT` | Null, misaligned, out-of-range, or unknown argument |
| `MADOPILOT_STATUS_UNSUPPORTED` | Valid request for a capability unavailable in this build/target |
| `MADOPILOT_STATUS_CANCELLED` | Cancellation token triggered before commit |
| `MADOPILOT_STATUS_DEADLINE_EXCEEDED` | Monotonic deadline expired before commit |
| `MADOPILOT_STATUS_CLOSED` | Session or engine is closed |
| `MADOPILOT_STATUS_TARGET_LOST` | Capture or input target no longer exists |
| `MADOPILOT_STATUS_LIMIT_EXCEEDED` | Configured or implementation safety ceiling exceeded |
| `MADOPILOT_STATUS_CAPTURE_FAILED` | Capture stream or device failed |
| `MADOPILOT_STATUS_ASSET_INVALID` | Asset package violated a manifest, path, digest, or ceiling rule |
| `MADOPILOT_STATUS_VISION_FAILED` | Matching or OCR backend failed |
| `MADOPILOT_STATUS_INTERNAL` | Internal library invariant failed |
| `MADOPILOT_STATUS_INTERNAL_PANIC` | Rust panic contained at the C ABI boundary |
| `MADOPILOT_STATUS_INPUT_FAILED` | Input request refused before admission (no receipt created) |

Key status and geometry rules:

- **Asset fault and stage detail:** `madopilot_error_detail_t` sets
  `MADOPILOT_ERROR_HAS_ASSET_DETAIL` with `asset_fault` and `asset_stage` both
  for invalid packages (`MADOPILOT_STATUS_ASSET_INVALID`) and when requesting an
  undeclared template from a valid package (`MADOPILOT_STATUS_INVALID_ARGUMENT`
  with `MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` at `MADOPILOT_ASSET_STAGE_COMMIT`).
- **Capture-pixel regions:** Caller-supplied C regions (`map`, `find`, and OCR)
  must use `MADOPILOT_SPACE_CAPTURE_PIXELS`; other spaces return
  `MADOPILOT_STATUS_INVALID_ARGUMENT`.
- **Per-call error ownership:** Errors are returned exclusively through the
  per-call `out_error` handle; there is no thread-local or global last-error slot.

## Singular and grouped OCR with immutable results

`session_recognize` and `session_scan_ocr_zones` borrow the session, exact frame,
request/zone views, and operation record for the synchronous call, committing an
independent immutable `madopilot_ocr_result_t` or
`madopilot_ocr_zone_scan_result_t` on success.

Engine construction options remain separated in dedicated structs so frozen
records never change size:

- `madopilot_default_ocr_options_t` (`engine_create_with_default_ocr`, ABI 1.3):
  canonical model-root and runtime paths for the native `G-004` profile.
- `madopilot_ocr_profile_options_t` (`engine_create_with_ocr_profile`, ABI 1.4):
  explicit `MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR` selection (`MADOPILOT_OCR_PROFILE_G004`
  is additionally accepted by ABI 1.5 provider-policy construction).
- `madopilot_ocr_provider_options_t` (`engine_create_with_ocr_provider`, ABI 1.5):
  adds the execution-provider policy and optional CUDA dependency root.

## Native capabilities and non-prompting permissions

- `engine_capabilities`: Reports whether the engine supports input, permission
  probes, and OCR (`8` bytes).
- `engine_ocr_descriptor` (ABI 1.4, `88` bytes) and
  `engine_ocr_provider_descriptor` (ABI 1.5, `40` bytes): Report the configured
  OCR model/profile and provider descriptors, or `MADOPILOT_STATUS_UNSUPPORTED`
  when unconfigured.
- `target_list_input_capability`, `engine_input_descriptor`, and
  `session_input_descriptor`: Report per-pair input compatibility (`supported_pairs`,
  `unknown_pairs`, `known_pairs`), address scope, focus requirement, accepted
  pointer spaces, permission kind, and submission evidence.
- `engine_permission`: Non-prompting probe on macOS for Screen Recording and
  `InputControl` (`CGPreflightPostEventAccess`). On Windows,
  `MADOPILOT_ENGINE_READS_PERMISSIONS` is clear and `engine_permission` returns
  `MADOPILOT_STATUS_UNSUPPORTED`.
- Target and stream `uint64_t` IDs are engine-local ordinals and are compared
  only within the engine that issued them.

## Input admission, submission evidence, and receipts

Input operation kind (`pointer`, `keyboard`, `text`) and delivery route
(`SYSTEM`, `WINDOW_MESSAGE`, `PROCESS_DIRECTED`) are orthogonal axes, paired with
explicit `SubmissionEvidence` (`INVOCATION_ONLY`, `SYSTEM_INPUT_ADMISSION`,
`TARGET_QUEUE_ADMISSION`, `TARGET_PROTOCOL_ACKNOWLEDGEMENT`). `session_open_with_input`
opens a session with an explicit input policy without altering the frozen
`madopilot_open_request_t`.

Published C ABI input ceilings:

| Contract | Ceiling |
|---|---:|
| Events per sequence (`MADOPILOT_INPUT_MAX_EVENTS`) | `256` |
| Text per event | `4,096` Unicode scalars / `16,384` UTF-8 bytes |
| Delay per event | `5,000,000,000` ns |
| Scroll delta per axis | `[-120, 120]` (not both zero) |
| Function key number / `MADOPILOT_KEY_CHARACTER` | `1..=24` / 1 non-control Unicode scalar |
| Cleanup budget | `256` releases and `250,000,000` ns |

Pre-admission refusals return `MADOPILOT_STATUS_INPUT_FAILED` with no receipt.
Once admitted, `session_send_input` returns `MADOPILOT_STATUS_OK` with one
immutable `madopilot_input_receipt_t` (`COMPLETE`, `PARTIAL`, or `UNEXECUTED`)
inspected via `input_receipt_info` and `input_receipt_attempt_at`. Only
`NOT_NEEDED` and `COMPLETE` cleanup states prove no sequence-pressed state
remains held. See [Windows input verification](windows-input-verification.md#capability-boundary)
and [macOS input verification](macos-input-verification.md#capability-boundary)
for platform route matrices and qualification details.

## Bounded diagnostic stream

Configured via `engine_create_with_options`: `OFF` requires capacity `0`;
`NORMAL` and `DEBUG` require capacity `1..=65,536` (`MADOPILOT_STATUS_LIMIT_EXCEEDED`
above `65,536`). `engine_take_diagnostic_reader` yields one retainable reader per
engine, and `diagnostic_reader_drain` returns `BATCH`, `OPEN_EMPTY`, or
`END_OF_STREAM`. See [Bounded diagnostic observation](architecture.md#bounded-diagnostic-observation)
for the closed privacy-reviewed record schema and loss-accounting rules.

## Panic containment

Every exported entry point catches Rust panics before crossing into C, returning
`MADOPILOT_STATUS_INTERNAL_PANIC` and resetting valid outputs to their failure
states. `mado-pilot-capi` enforces an unwinding profile at compile time with
`#[cfg(panic = "abort")] compile_error!`; do not build this crate with
`-C panic=abort` or `-Z build-std` `panic_immediate_abort`.

## What ABI 1.2 does not contain

The frozen ABI 1.2 prefix ends at diagnostic batch access (`592` bytes) and
contains no OCR entries (added in 1.3–1.5), watcher entries, callbacks, or
general coordinate-conversion helpers.

## Building against the library

### Artifacts

| Target | Development build output | Reserved ABI-major release name (`G-007`/`G-012`) |
|---|---|---|
| `aarch64-apple-darwin` | `libmadopilot.dylib` | `libmadopilot.1.dylib` |
| `x86_64-pc-windows-msvc` | `madopilot.dll`, `madopilot.dll.lib` | `madopilot-1.dll`, `madopilot-1.lib` |

No `staticlib` is emitted while [`G-008`](validation-gates.md#g-008) remains open.

### Prerequisites

Running a C or C++ consumer requires the same host OpenCV 4 installation as Rust
(plus canonical `MADO_PILOT_G004_MODEL_ROOT` and `MADO_PILOT_ONNX_RUNTIME` paths
when running real OCR examples), a C11/C++17 compiler (`cc`/`c++` from Xcode
Command Line Tools on macOS, `cl` from an x64 MSVC Developer Command Prompt on
Windows), and **CMake 3.22+**. On Windows, `c-abi-check` strips `\\?\`
extended-length prefixes before invoking `cl` to avoid `C1083`. See
[native development prerequisites](../CONTRIBUTING.md#native-development-prerequisites)
for `tools/setup-native.py` usage.

### Compiling

On macOS, build the library and compile the introductory samples:

```sh
python3 tools/setup-native.py -- cargo build --locked --package mado-pilot-capi
cc -std=c11 -I crates/bindings/capi/include \
   crates/bindings/capi/examples/c/direct-abi.c \
   target/debug/libmadopilot.dylib -Wl,-rpath,"$PWD/target/debug" \
   -o target/debug/direct-abi-c
c++ -std=c++17 -I crates/bindings/capi/include \
   crates/bindings/capi/examples/cpp/direct-abi.cpp \
   target/debug/libmadopilot.dylib -Wl,-rpath,"$PWD/target/debug" \
   -o target/debug/direct-abi-cpp
target/debug/direct-abi-c
target/debug/direct-abi-cpp
```

On Windows, link `target\debug\madopilot.dll.lib` and include `target\debug` on
`PATH`. For CMake consumers (C or C++), import the source-tree package:

```cmake
cmake_minimum_required(VERSION 3.22)
project(my_app LANGUAGES CXX)
set(MADOPILOT_ARTIFACT_DIR "<absolute Cargo profile directory>")
add_subdirectory("<MadoPilot source>/crates/bindings/capi" madopilot)
add_executable(my_app main.cpp)
target_compile_features(my_app PRIVATE cxx_std_17)
target_link_libraries(my_app PRIVATE MadoPilot::C)
```

`crates/bindings/capi/CMakeLists.txt` enables no language or compiler of its own,
so C-only consumers can declare `project(my_app LANGUAGES C)` and link
`MadoPilot::C` without a C++ compiler.

For native input C verification flows (`--windows-native-fixture`,
`windows-native-input.exe`, and `macos-native-input`), see
[Windows input verification](windows-input-verification.md#explicit-c-abi-boundary-checks)
and [macOS input verification](macos-input-verification.md#explicit-c-abi-boundary-checks).

## How the header is verified

`include/madopilot/madopilot.h` is hand-written and verified against the Rust
`#[repr(C)]` definitions and all frozen ABI prefixes by step 8 of the
verification sequence:

```sh
python3 tools/setup-native.py -- cargo build --locked --package mado-pilot-capi
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
python3 tools/setup-native.py -- cargo build --locked --package mado-pilot-capi --features private-fixture --target-dir target/c-abi-private-fixture
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot-capi --features private-fixture --target-dir target/c-abi-private-fixture --example c-abi-check -- --label "<host>"
```

`c-abi-check` compares every struct size, alignment, and field offset between
`rustc` and the target C compiler, compiles and executes the frozen ABI `1.0`,
`1.2`, `1.3`, and `1.4` callers against the ABI `1.5` library, runs both C/C++
and C-only CMake consumers, and exercises deterministic replay and OCR flows.

## Released header fixtures

Every frozen ABI minor has an immutable header and caller under
[`crates/bindings/capi/tests/abi-compat/`](../crates/bindings/capi/tests/abi-compat/README.md).
New ABI minors add a new fixture directory rather than editing released headers.
