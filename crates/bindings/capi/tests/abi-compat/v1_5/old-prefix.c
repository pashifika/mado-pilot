/*
 * Frozen ABI 1.5 consumer: only this directory's 736-byte header is visible.
 * Exercises every declared entry, the deterministic matching flow, retained
 * children after parent teardown, and initialized input/OCR/provider refusals.
 * No native input, external OCR models, or OCR runtime loading is requested.
 *
 * Frozen with the header. Do not add later-minor behavior here.
 *   usage: madopilot-abi-compat-v1_5 --package <dir>
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "deterministic-scene.h"
#include "madopilot/madopilot.h"

static int failures = 0;

static madopilot_str_t borrow(const char* text)
{
    madopilot_str_t view;
    view.data = text;
    view.len = strlen(text);
    return view;
}

static int expect(int condition, const char* what)
{
    if (!condition) {
        printf("FAIL: %s\n", what);
        failures += 1;
    }
    return condition;
}

static int expect_ok(madopilot_status_t status, const char* what)
{
    return expect(status == MADOPILOT_STATUS_OK, what);
}

static int equals(madopilot_str_t view, const char* text)
{
    return view.len == strlen(text) && view.data != NULL &&
           memcmp(view.data, text, view.len) == 0;
}

static int same_stamp(const madopilot_frame_stamp_t* a,
                      const madopilot_frame_stamp_t* b)
{
    return a->stream == b->stream && a->epoch == b->epoch &&
           a->sequence == b->sequence && a->geometry == b->geometry;
}

static void expect_error(const madopilot_api_t* api, madopilot_error_t** error,
                         madopilot_status_t status)
{
    madopilot_error_detail_t detail;
    if (!expect(*error != NULL, "a refusal returns an owned structured error")) {
        return;
    }
    expect_ok(api->error_retain(*error), "error_retain");
    expect_ok(api->error_release(*error), "releasing one error reference");
    memset(&detail, 0, sizeof(detail));
    detail.struct_size = (uint32_t)sizeof(detail);
    if (expect_ok(api->error_describe(*error, &detail), "error_describe")) {
        expect(detail.status == status, "the retained error preserves its status");
        if (status == MADOPILOT_STATUS_INVALID_ARGUMENT) {
            expect(detail.category == MADOPILOT_ERROR_CATEGORY_ABI,
                   "malformed foreign input is an ABI error");
        }
    }
    expect_ok(api->error_release(*error), "error_release");
    *error = NULL;
}

static int negotiate_at_the_mandatory_prefix(void)
{
    const madopilot_api_t* api = NULL;
    madopilot_build_info_t build;
    madopilot_str_t text;
    if (!expect_ok(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                                     MADOPILOT_API_SIZE_INFORMATION, &api),
                   "negotiating at the mandatory prefix") ||
        !expect(api != NULL, "a successful negotiation returns a table")) {
        return 0;
    }
    expect(api->struct_size >= (uint32_t)sizeof(madopilot_api_t),
           "minor 5 reports the library's size, not the caller's prefix");
    expect(api->abi_major == MADOPILOT_ABI_MAJOR &&
               api->abi_minor >= MADOPILOT_ABI_MINOR,
           "the library supports the frozen version");
    memset(&build, 0, sizeof(build));
    build.struct_size = (uint32_t)sizeof(build);
    if (expect_ok(api->describe_build(&build), "describe_build")) {
        expect(build.abi_major == api->abi_major &&
                   build.abi_minor == api->abi_minor &&
                   build.table_size == api->struct_size,
               "build information agrees with negotiation");
    }
    text = borrow("sentinel");
    expect_ok(api->status_text(MADOPILOT_STATUS_OK, &text), "status_text");
    expect(equals(text, "ok"), "the released success slug is unchanged");
    return 1;
}

static void negotiation_refusals(void)
{
    const madopilot_api_t* api = (const madopilot_api_t*)(void*)&failures;
    expect(madopilot_get_api(MADOPILOT_ABI_MAJOR + 1u, MADOPILOT_ABI_MINOR,
                             sizeof(madopilot_api_t), &api) ==
               MADOPILOT_STATUS_UNSUPPORTED,
           "a different ABI major is refused");
    expect(api == NULL, "a refused major nulls its table output");
    api = (const madopilot_api_t*)(void*)&failures;
    expect(madopilot_get_api(MADOPILOT_ABI_MAJOR, 1u,
                             sizeof(madopilot_api_t), &api) ==
               MADOPILOT_STATUS_UNSUPPORTED,
           "the unreleased minor 1 remains unsupported");
    expect(api == NULL, "a refused minor nulls its table output");
    api = (const madopilot_api_t*)(void*)&failures;
    expect(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                             MADOPILOT_API_SIZE_INFORMATION - 1u, &api) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "a size below the mandatory prefix is refused");
    expect(api == NULL, "a refused extent nulls its table output");
}

/* All source storage is still live; malformed options must publish no engine. */
static void check_ocr_constructors(const madopilot_api_t* api,
                                   const madopilot_source_t* source,
                                   const madopilot_operation_t* operation,
                                   madopilot_engine_t* engine,
                                   const char* package_dir)
{
    madopilot_ocr_profile_options_t profile;
    madopilot_ocr_provider_options_t provider;
    madopilot_engine_t* candidate = engine;
    madopilot_error_t* error = NULL;
    madopilot_cancellation_t* cancellation = NULL;
    madopilot_operation_t cancelled = *operation;
    int32_t is_cancelled = -1;

    expect(api->engine_create_with_default_ocr(
               source, NULL, NULL, operation, &candidate, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "default OCR requires its prerequisite record");
    expect(candidate == NULL, "default OCR initializes its owner output");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);

    memset(&profile, 0, sizeof(profile));
    profile.struct_size = (uint32_t)sizeof(profile);
    profile.kind = MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR;
    candidate = engine;
    expect(api->engine_create_with_ocr_profile(
               source, NULL, &profile, operation, &candidate, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "explicit OCR requires nonempty controlled prerequisite views");
    expect(candidate == NULL, "profile OCR initializes its owner output");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);

    /* Nonempty views reach provider validation without opening either path. */
    profile.model_root = borrow(package_dir);
    profile.runtime_path = borrow(package_dir);
    memset(&provider, 0, sizeof(provider));
    provider.struct_size = (uint32_t)sizeof(provider);
    provider.policy = MADOPILOT_OCR_PROVIDER_POLICY_CPU;
    candidate = engine;
    expect(api->engine_create_with_ocr_provider(
               source, NULL, &profile, NULL, operation, &candidate, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "ABI 1.5 provider construction requires its provider record");
    expect(candidate == NULL, "missing provider publishes no engine");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);

    candidate = engine;
    provider.struct_size = (uint32_t)offsetof(madopilot_ocr_provider_options_t,
                                             provider_root);
    expect(api->engine_create_with_ocr_provider(
               source, NULL, &profile, &provider, operation, &candidate, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "provider options require the complete released prefix");
    expect(candidate == NULL, "a short provider record publishes no engine");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);

    candidate = engine;
    provider.struct_size = (uint32_t)sizeof(provider);
    provider.policy = 0;
    expect(api->engine_create_with_ocr_provider(
               source, NULL, &profile, &provider, operation, &candidate, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "an unknown provider policy is not an implicit CPU default");
    expect(candidate == NULL, "unknown provider policy publishes no engine");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);

    candidate = engine;
    provider.policy = MADOPILOT_OCR_PROVIDER_POLICY_CPU;
    provider.provider_root = borrow(package_dir);
    expect(api->engine_create_with_ocr_provider(
               source, NULL, &profile, &provider, operation, &candidate, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "CPU policy refuses an accelerator dependency root");
    expect(candidate == NULL, "incompatible provider configuration publishes nothing");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);
    provider.provider_root.data = NULL;
    provider.provider_root.len = 0;

    if (expect_ok(api->cancellation_create(&cancellation), "cancellation_create")) {
        expect_ok(api->cancellation_retain(cancellation), "cancellation_retain");
        expect_ok(api->cancellation_release(cancellation),
                  "releasing one cancellation reference");
        expect_ok(api->cancellation_is_cancelled(cancellation, &is_cancelled),
                  "cancellation_is_cancelled before cancellation");
        expect(is_cancelled == 0, "a new cancellation owner is not cancelled");
        expect_ok(api->cancellation_cancel(cancellation), "cancellation_cancel");
        expect_ok(api->cancellation_cancel(cancellation), "cancellation is idempotent");
        expect_ok(api->cancellation_is_cancelled(cancellation, &is_cancelled),
                  "cancellation_is_cancelled after cancellation");
        expect(is_cancelled == 1, "the surviving reference observes cancellation");
        cancelled.cancellation = cancellation;
        candidate = engine;
        expect(api->engine_create_with_ocr_provider(
                   source, NULL, &profile, &provider, &cancelled, &candidate,
                   &error) == MADOPILOT_STATUS_CANCELLED,
               "provider cancellation refuses before loading prerequisites");
        expect(candidate == NULL, "cancelled provider construction publishes nothing");
        expect_error(api, &error, MADOPILOT_STATUS_CANCELLED);
        expect_ok(api->cancellation_release(cancellation), "cancellation_release");
    }
}

static void check_diagnostics(const madopilot_api_t* api,
                              const madopilot_source_t* source,
                              const madopilot_operation_t* operation)
{
    madopilot_engine_options_t options;
    madopilot_engine_t* engine = NULL;
    madopilot_diagnostic_reader_t* reader = NULL;
    madopilot_diagnostic_reader_t* second = NULL;
    madopilot_diagnostic_batch_t* batch = NULL;
    madopilot_diagnostic_batch_t* next = NULL;
    madopilot_diagnostic_drain_state_t state = 0;
    madopilot_diagnostic_batch_info_t info;
    madopilot_diagnostic_record_t record;
    madopilot_permission_t permission;
    madopilot_error_t* error = NULL;
    size_t index;
    int saw_permission = 0;

    memset(&options, 0, sizeof(options));
    options.struct_size = (uint32_t)sizeof(options);
    options.diagnostic_level = MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL;
    options.diagnostic_capacity = 8;
    if (!expect_ok(api->engine_create_with_options(
                       source, &options, operation, &engine, &error),
                   "engine_create_with_options")) {
        goto cleanup;
    }
    if (!expect_ok(api->engine_take_diagnostic_reader(engine, &reader),
                   "engine_take_diagnostic_reader") ||
        !expect(reader != NULL, "enabled diagnostics yield a reader")) {
        goto cleanup;
    }
    second = reader;
    expect_ok(api->engine_take_diagnostic_reader(engine, &second),
              "diagnostic reader can be taken only once");
    expect(second == NULL, "a second take initializes its owner output");
    expect_ok(api->diagnostic_reader_retain(reader), "diagnostic_reader_retain");
    expect_ok(api->diagnostic_reader_release(reader),
              "releasing one diagnostic reader reference");

    memset(&permission, 0xff, sizeof(permission));
    permission.struct_size = (uint32_t)sizeof(permission);
    expect(api->engine_permission(engine, MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE,
                                   operation, &permission, &error) ==
               MADOPILOT_STATUS_UNSUPPORTED,
           "replay does not probe native permission");
    expect(permission.state == MADOPILOT_PERMISSION_STATE_UNKNOWN &&
               permission.context.data == NULL && permission.context.len == 0,
           "an unsupported permission read initializes its output");
    expect_error(api, &error, MADOPILOT_STATUS_UNSUPPORTED);
    expect_ok(api->engine_release(engine), "releasing the diagnostic producer");
    engine = NULL;

    if (!expect_ok(api->diagnostic_reader_drain(reader, &state, &batch),
                   "diagnostic_reader_drain") ||
        !expect(state == MADOPILOT_DIAGNOSTIC_DRAIN_BATCH && batch != NULL,
                "retained diagnostics preserve the permission outcome")) {
        goto cleanup;
    }
    expect_ok(api->diagnostic_batch_retain(batch), "diagnostic_batch_retain");
    expect_ok(api->diagnostic_batch_release(batch),
              "releasing one diagnostic batch reference");
    next = batch;
    expect_ok(api->diagnostic_reader_drain(reader, &state, &next),
              "draining the sealed diagnostic stream");
    expect(state == MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM && next == NULL,
           "a fully drained released producer reaches end of stream");
    expect_ok(api->diagnostic_reader_release(reader), "diagnostic_reader_release");
    reader = NULL;

    memset(&info, 0, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    if (expect_ok(api->diagnostic_batch_info(batch, &info), "diagnostic_batch_info")) {
        expect(info.discarded_normal == 0 && info.discarded_debug == 0,
               "the single permission outcome fits without diagnostic loss");
        for (index = 0; index < info.record_count; index += 1) {
            memset(&record, 0, sizeof(record));
            record.struct_size = (uint32_t)sizeof(record);
            if (expect_ok(api->diagnostic_batch_record_at(batch, index, &record),
                          "diagnostic_batch_record_at") &&
                record.kind == MADOPILOT_DIAGNOSTIC_KIND_PERMISSION) {
                expect(record.permission_kind == MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE &&
                           (record.flags & MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS) != 0 &&
                           record.status == MADOPILOT_STATUS_UNSUPPORTED,
                       "the retained record preserves the typed permission refusal");
                saw_permission = 1;
            }
        }
        expect(saw_permission, "the batch remains readable after engine and reader release");
        memset(&record, 0xff, sizeof(record));
        record.struct_size = (uint32_t)sizeof(record);
        expect(api->diagnostic_batch_record_at(batch, (size_t)info.record_count,
                                               &record) == MADOPILOT_STATUS_INVALID_ARGUMENT,
               "diagnostic indexes remain bounded");
        expect(record.sequence == 0 && record.flags == 0,
               "an invalid diagnostic index initializes its output");
    }

cleanup:
    api->error_release(error);
    api->diagnostic_batch_release(batch);
    api->diagnostic_reader_release(reader);
    api->engine_release(engine);
}

static void check_input(const madopilot_api_t* api, madopilot_engine_t* engine,
                        madopilot_target_list_t* targets,
                        madopilot_session_t* session,
                        const madopilot_open_request_t* open_request,
                        const madopilot_operation_t* operation, uint64_t target)
{
    madopilot_engine_capabilities_t capabilities;
    madopilot_input_capability_t capability;
    madopilot_input_descriptor_t descriptor;
    madopilot_input_open_request_t input_open;
    madopilot_session_t* optional = NULL;
    madopilot_input_receipt_t* receipt = (madopilot_input_receipt_t*)(void*)&failures;
    madopilot_input_receipt_info_t info;
    madopilot_input_attempt_t attempt;
    madopilot_error_t* error = NULL;
    size_t count = 99;

    memset(&capabilities, 0, sizeof(capabilities));
    capabilities.struct_size = (uint32_t)sizeof(capabilities);
    expect_ok(api->engine_capabilities(engine, &capabilities), "engine_capabilities");
    expect((capabilities.flags & (MADOPILOT_ENGINE_DELIVERS_INPUT |
                                 MADOPILOT_ENGINE_READS_PERMISSIONS |
                                 MADOPILOT_ENGINE_HAS_OCR)) == 0,
           "the replay engine has no implicit native input, permission, or OCR provider");
    memset(&capability, 0, sizeof(capability));
    capability.struct_size = (uint32_t)sizeof(capability);
    expect_ok(api->target_list_input_capability(
                  targets, 0, MADOPILOT_INPUT_OPERATION_POINTER,
                  MADOPILOT_INPUT_DELIVERY_SYSTEM, &capability),
              "target_list_input_capability");
    expect(capability.target == target &&
               capability.support == MADOPILOT_CAPABILITY_UNSUPPORTED,
           "replay advertises no system pointer route");
    memset(&descriptor, 0, sizeof(descriptor));
    descriptor.struct_size = (uint32_t)sizeof(descriptor);
    expect_ok(api->engine_input_descriptor(engine, targets, 0, operation,
                                           &descriptor, &error),
              "engine_input_descriptor");
    expect(descriptor.target == target && descriptor.supported_pairs == 0 &&
               descriptor.unknown_pairs == 0,
           "engine input description grants no replay route");
    memset(&input_open, 0, sizeof(input_open));
    input_open.struct_size = (uint32_t)sizeof(input_open);
    input_open.requirement = MADOPILOT_INPUT_OPTIONAL;
    expect_ok(api->session_open_with_input(engine, targets, 0, open_request,
                                           &input_open, operation, &optional, &error),
              "optional input preserves replay capture");
    expect_ok(api->session_close(optional, operation, &error), "closing optional-input capture");
    expect_ok(api->session_release(optional), "releasing optional-input capture");
    memset(&descriptor, 0, sizeof(descriptor));
    descriptor.struct_size = (uint32_t)sizeof(descriptor);
    expect_ok(api->session_input_descriptor(session, &descriptor), "session_input_descriptor");
    expect(descriptor.target == target && descriptor.supported_pairs == 0 &&
               descriptor.unknown_pairs == 0,
           "capture-only session accepts no implicit input route");
    expect(api->session_send_input(session, NULL, operation, &receipt, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "input submission requires a request before admission");
    expect(receipt == NULL, "refused input publishes no receipt");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);
    expect_ok(api->input_receipt_retain(NULL), "null input_receipt_retain");
    expect_ok(api->input_receipt_release(NULL), "null input_receipt_release");
    memset(&info, 0xff, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    expect(api->input_receipt_info(NULL, &info) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "a receipt description requires an owner");
    expect(info.attempt_count == 0 && info.submitted == 0,
           "missing receipt reports no submitted input");
    expect(api->input_receipt_attempt_count(NULL, &count) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "receipt attempt count requires an owner");
    expect(count == 0, "missing receipt initializes its attempt count");
    memset(&attempt, 0xff, sizeof(attempt));
    attempt.struct_size = (uint32_t)sizeof(attempt);
    expect(api->input_receipt_attempt_at(NULL, 0, &attempt) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "receipt attempt access requires an owner");
    expect(attempt.submitted == 0 && attempt.flags == 0,
           "missing receipt initializes its attempt output");
    api->error_release(error);
}

static void check_ocr_results(const madopilot_api_t* api, madopilot_engine_t* engine,
                              madopilot_session_t* session,
                              const madopilot_operation_t* operation)
{
    madopilot_ocr_result_t* result = (madopilot_ocr_result_t*)(void*)&failures;
    madopilot_ocr_zone_scan_result_t* grouped =
        (madopilot_ocr_zone_scan_result_t*)(void*)&failures;
    madopilot_error_t* error = NULL;
    madopilot_ocr_result_info_t info;
    madopilot_ocr_region_t region;
    madopilot_ocr_zone_scan_result_info_t group_info;
    madopilot_ocr_zone_result_t zone;
    madopilot_ocr_engine_descriptor_t descriptor;
    madopilot_str_t text;
    struct {
        madopilot_ocr_provider_descriptor_t value;
        uint64_t guard;
    } provider;

    expect(api->session_recognize(session, NULL, operation, &result, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "singular OCR requires a request");
    expect(result == NULL, "singular OCR initializes its owner output");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);
    expect_ok(api->ocr_result_retain(NULL), "null ocr_result_retain");
    expect_ok(api->ocr_result_release(NULL), "null ocr_result_release");
    memset(&info, 0xff, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    expect(api->ocr_result_info(NULL, &info) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "OCR info requires an owner");
    expect(info.region_count == 0 && info.model_id.data == NULL,
           "missing OCR info initializes its count and views");
    memset(&region, 0xff, sizeof(region));
    region.struct_size = (uint32_t)sizeof(region);
    expect(api->ocr_result_region_at(NULL, 0, &region) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "OCR region requires an owner");
    expect(region.confidence == 0.0, "missing OCR region initializes confidence");
    text = borrow("sentinel");
    expect(api->ocr_result_text_at(NULL, 0, &text) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "OCR text requires an owner");
    expect(text.data == NULL && text.len == 0, "missing OCR text initializes its view");

    expect(api->session_scan_ocr_zones(session, NULL, operation, &grouped, &error) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "grouped OCR requires a request");
    expect(grouped == NULL, "grouped OCR initializes its owner output");
    expect_error(api, &error, MADOPILOT_STATUS_INVALID_ARGUMENT);
    expect_ok(api->ocr_zone_scan_result_retain(NULL), "null ocr_zone_scan_result_retain");
    expect_ok(api->ocr_zone_scan_result_release(NULL), "null ocr_zone_scan_result_release");
    memset(&group_info, 0xff, sizeof(group_info));
    group_info.struct_size = (uint32_t)sizeof(group_info);
    expect(api->ocr_zone_scan_result_info(NULL, &group_info) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "grouped OCR info requires an owner");
    expect(group_info.zone_count == 0, "missing grouped OCR info initializes its count");
    memset(&zone, 0xff, sizeof(zone));
    zone.struct_size = (uint32_t)sizeof(zone);
    expect(api->ocr_zone_scan_result_zone_at(NULL, 0, &zone) == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "grouped OCR zone requires an owner");
    expect(zone.region_count == 0, "missing grouped OCR zone initializes its count");
    memset(&region, 0xff, sizeof(region));
    region.struct_size = (uint32_t)sizeof(region);
    expect(api->ocr_zone_scan_result_region_at(NULL, 0, 0, &region) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "grouped OCR region requires an owner");
    expect(region.confidence == 0.0, "missing grouped OCR region initializes confidence");
    text = borrow("sentinel");
    expect(api->ocr_zone_scan_result_text_at(NULL, 0, 0, &text) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "grouped OCR text requires an owner");
    expect(text.data == NULL && text.len == 0, "missing grouped OCR text initializes its view");
    memset(&descriptor, 0xff, sizeof(descriptor));
    descriptor.struct_size = (uint32_t)sizeof(descriptor);
    expect(api->engine_ocr_descriptor(engine, &descriptor) == MADOPILOT_STATUS_UNSUPPORTED,
           "an engine without OCR has no OCR descriptor");
    expect(descriptor.backend_id.data == NULL && descriptor.backend_id.len == 0 &&
               descriptor.model_id.data == NULL && descriptor.profile_id.data == NULL,
           "an unavailable OCR descriptor initializes its borrowed views");

    memset(&provider, 0xff, sizeof(provider));
    provider.value.struct_size = (uint32_t)sizeof(provider.value);
    expect(api->engine_ocr_provider_descriptor(engine, &provider.value) ==
               MADOPILOT_STATUS_UNSUPPORTED,
           "ABI 1.5 does not invent a CPU provider for an engine without OCR");
    expect(provider.value.struct_size == sizeof(provider.value) &&
               provider.value.flags == 0 && provider.value.requested_policy == 0 &&
               provider.value.active_provider == MADOPILOT_OCR_EXECUTION_PROVIDER_UNSPECIFIED &&
               provider.value.initialization_fell_back == 0 &&
               provider.value.fallback_reason == MADOPILOT_OCR_PROVIDER_FALLBACK_NONE &&
               provider.value.runtime_profile.data == NULL && provider.value.runtime_profile.len == 0,
           "an unavailable provider descriptor resets all facts and its borrowed view");
    expect(provider.guard == UINT64_MAX, "provider descriptor respects the frozen output extent");
    memset(&provider.value, 0xff, sizeof(provider.value));
    provider.value.struct_size = (uint32_t)sizeof(provider.value);
    expect(api->engine_ocr_provider_descriptor(NULL, &provider.value) ==
               MADOPILOT_STATUS_INVALID_ARGUMENT,
           "provider descriptor access requires an engine owner");
    expect(provider.value.flags == 0 && provider.value.runtime_profile.data == NULL &&
               provider.value.runtime_profile.len == 0 && provider.guard == UINT64_MAX,
           "invalid provider descriptor access initializes only its declared output");
}

/* Same scene and match oracle as v1_4, with every inherited entry exercised. */
static int run_the_flow(const madopilot_api_t* api, const char* package_dir)
{
    madopilot_operation_t operation;
    madopilot_replay_frame_t frame_input;
    madopilot_source_t source;
    madopilot_package_source_t package_source;
    madopilot_open_request_t open_request;
    madopilot_find_request_t find_request;
    madopilot_map_request_t map_request;
    madopilot_result_info_t result_info;
    madopilot_target_t target;
    madopilot_session_info_t session_info;
    madopilot_frame_info_t frame_info;
    madopilot_frame_stamp_t frame_stamp;
    madopilot_frame_stamp_t retained_stamp;
    madopilot_package_info_t package_info;
    madopilot_template_info_t template_info;
    madopilot_match_options_t options;
    madopilot_image_t image;
    madopilot_str_t template_id;
    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_mapping_t* mapping = NULL;
    madopilot_package_t* package = NULL;
    madopilot_template_t* prepared = NULL;
    madopilot_result_t* result = NULL;
    madopilot_error_t* error = NULL;
    uint8_t* scene = NULL;
    uint8_t pixel[3];
    uint64_t now = 0;
    size_t index;
    size_t count = 0;
    int32_t closed = -1;
    int found[2] = { 0, 0 };
    int completed = 0;

    if (!expect_ok(api->clock_now(&now), "clock_now")) {
        return 0;
    }
    memset(&operation, 0, sizeof(operation));
    operation.struct_size = (uint32_t)sizeof(operation);
    operation.flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    operation.deadline_nanos = now + 30ull * 1000ull * 1000ull * 1000ull;
    scene = (uint8_t*)malloc(SCENE_BYTES);
    if (!expect(scene != NULL, "allocating the deterministic scene")) {
        goto cleanup;
    }
    scene_fill_rgba(scene);
    memset(&frame_input, 0, sizeof(frame_input));
    frame_input.struct_size = (uint32_t)sizeof(frame_input);
    frame_input.width = SCENE_WIDTH;
    frame_input.height = SCENE_HEIGHT;
    frame_input.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    frame_input.continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
    frame_input.pixels.data = scene;
    frame_input.pixels.len = SCENE_BYTES;
    memset(&source, 0, sizeof(source));
    source.struct_size = (uint32_t)sizeof(source);
    source.kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
    source.frames = &frame_input;
    source.frame_count = 1;
    source.frame_stride = sizeof(frame_input);
    source.target_name = borrow("panel");
    if (!expect_ok(api->engine_create(&source, &operation, &engine, &error), "engine_create")) {
        goto cleanup;
    }
    check_ocr_constructors(api, &source, &operation, engine, package_dir);
    check_diagnostics(api, &source, &operation);
    free(scene);
    scene = NULL;
    expect_ok(api->engine_retain(engine), "engine_retain");
    expect_ok(api->engine_release(engine), "releasing one engine reference");
    if (!expect_ok(api->engine_discover(engine, &operation, &targets, &error), "engine_discover")) {
        goto cleanup;
    }
    expect_ok(api->target_list_retain(targets), "target_list_retain");
    expect_ok(api->target_list_release(targets), "releasing one target list reference");
    expect_ok(api->target_list_count(targets, &count), "target_list_count");
    expect(count == 1, "the replay source declares one target");
    memset(&target, 0, sizeof(target));
    target.struct_size = (uint32_t)sizeof(target);
    expect_ok(api->target_list_get(targets, 0, &target), "target_list_get");
    expect(target.width == SCENE_WIDTH && target.height == SCENE_HEIGHT &&
               equals(target.name, "panel"), "the target describes the supplied scene");
    memset(&open_request, 0, sizeof(open_request));
    open_request.struct_size = (uint32_t)sizeof(open_request);
    open_request.flags = MADOPILOT_OPEN_HAS_REQUIRED_FORMAT;
    open_request.required_format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    if (!expect_ok(api->session_open(engine, targets, 0, &open_request, &operation,
                                     &session, &error), "session_open")) {
        goto cleanup;
    }
    check_input(api, engine, targets, session, &open_request, &operation, target.target);
    api->target_list_release(targets);
    targets = NULL;
    expect_ok(api->session_retain(session), "session_retain");
    expect_ok(api->session_release(session), "releasing one session reference");
    memset(&session_info, 0, sizeof(session_info));
    session_info.struct_size = (uint32_t)sizeof(session_info);
    expect_ok(api->session_describe(session, &session_info), "session_describe");
    expect(session_info.width == SCENE_WIDTH && session_info.height == SCENE_HEIGHT &&
               session_info.accepts_input == 0, "session retains capture without implicit input");
    expect_ok(api->session_is_closed(session, &closed), "session_is_closed before close");
    expect(closed == 0, "a newly opened session accepts work");
    if (!expect_ok(api->session_acquire_frame(session, &operation, &frame, &error),
                   "session_acquire_frame")) {
        goto cleanup;
    }
    expect_ok(api->frame_retain(frame), "frame_retain");
    expect_ok(api->frame_release(frame), "releasing one frame reference");
    memset(&frame_stamp, 0, sizeof(frame_stamp));
    frame_stamp.struct_size = (uint32_t)sizeof(frame_stamp);
    expect_ok(api->frame_stamp(frame, &frame_stamp), "frame_stamp");
    expect(frame_stamp.stream == session_info.stream && frame_stamp.epoch == 0 &&
               frame_stamp.sequence == 0 && frame_stamp.geometry == 0,
           "the acquired static frame belongs to its session");
    memset(&frame_info, 0, sizeof(frame_info));
    frame_info.struct_size = (uint32_t)sizeof(frame_info);
    expect_ok(api->frame_describe(frame, &frame_info), "frame_describe");
    expect(frame_info.width == SCENE_WIDTH && frame_info.height == SCENE_HEIGHT &&
               frame_info.format == MADOPILOT_PIXEL_FORMAT_RGBA8,
           "frame geometry preserves the released format");
    memset(&map_request, 0, sizeof(map_request));
    map_request.struct_size = (uint32_t)sizeof(map_request);
    map_request.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    if (!expect_ok(api->frame_map(frame, &map_request, &operation, &mapping, &error), "frame_map")) {
        goto cleanup;
    }
    expect_ok(api->mapping_retain(mapping), "mapping_retain");
    expect_ok(api->mapping_release(mapping), "releasing one mapping reference");
    memset(&package_source, 0, sizeof(package_source));
    package_source.struct_size = (uint32_t)sizeof(package_source);
    package_source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
    package_source.path = borrow(package_dir);
    if (!expect_ok(api->package_load(engine, &package_source, &operation, &package, &error),
                   "package_load")) {
        goto cleanup;
    }
    expect_ok(api->package_retain(package), "package_retain");
    expect_ok(api->package_release(package), "releasing one package reference");
    memset(&package_info, 0, sizeof(package_info));
    package_info.struct_size = (uint32_t)sizeof(package_info);
    expect_ok(api->package_describe(package, &package_info), "package_describe");
    expect(package_info.template_count == 2 &&
               equals(package_info.package_id, "madopilot.example.phase1-slice"),
           "the frozen caller reads the complete package identity");
    for (index = 0; index < package_info.template_count; index += 1) {
        template_id = borrow("sentinel");
        expect_ok(api->package_template_id(package, index, &template_id), "package_template_id");
        if (equals(template_id, "panel.patch")) {
            found[0] = 1;
        } else if (equals(template_id, "panel.absent")) {
            found[1] = 1;
        }
    }
    expect(found[0] && found[1], "both declared template identities remain addressable");
    found[0] = found[1] = 0;
    if (!expect_ok(api->template_prepare_from_package(engine, package, borrow("panel.patch"),
                                                     &operation, &prepared, &error),
                   "template_prepare_from_package")) {
        goto cleanup;
    }
    expect_ok(api->template_retain(prepared), "template_retain");
    expect_ok(api->template_release(prepared), "releasing one template reference");
    api->package_release(package);
    package = NULL;
    memset(&template_info, 0, sizeof(template_info));
    template_info.struct_size = (uint32_t)sizeof(template_info);
    expect_ok(api->template_describe(prepared, &template_info), "template_describe");
    expect(template_info.width == PATCH_WIDTH && template_info.height == PATCH_HEIGHT &&
               equals(template_info.id, "panel.patch"),
           "the prepared template remains readable after package release");
    memset(&find_request, 0, sizeof(find_request));
    find_request.struct_size = (uint32_t)sizeof(find_request);
    find_request.frame = frame;
    find_request.tmpl = prepared;
    if (!expect_ok(api->session_find(session, &find_request, &operation, &result, &error),
                   "session_find")) {
        goto cleanup;
    }
    expect_ok(api->result_retain(result), "result_retain");
    expect_ok(api->result_release(result), "releasing one result reference");
    memset(&options, 0, sizeof(options));
    options.struct_size = (uint32_t)sizeof(options);
    expect_ok(api->result_options(result, &options), "result_options");
    expect(options.min_score == template_info.min_score &&
               options.max_results == template_info.max_results &&
               (options.flags & (MADOPILOT_MATCH_HAS_MIN_SCORE | MADOPILOT_MATCH_HAS_MAX_RESULTS |
                                 MADOPILOT_MATCH_HAS_SUPPRESSION)) ==
                   (MADOPILOT_MATCH_HAS_MIN_SCORE | MADOPILOT_MATCH_HAS_MAX_RESULTS |
                    MADOPILOT_MATCH_HAS_SUPPRESSION),
           "the search reports the prepared template's effective defaults");
    check_ocr_results(api, engine, session, &operation);
    expect_ok(api->session_close(session, &operation, &error), "session_close");
    expect_ok(api->session_close(session, &operation, &error), "session_close is idempotent");
    expect_ok(api->session_is_closed(session, &closed), "session_is_closed after close");
    expect(closed == 1, "close changes the observable session state");
    api->template_release(prepared);
    prepared = NULL;
    api->frame_release(frame);
    frame = NULL;
    api->session_release(session);
    session = NULL;
    api->engine_release(engine);
    engine = NULL;

    memset(&retained_stamp, 0, sizeof(retained_stamp));
    retained_stamp.struct_size = (uint32_t)sizeof(retained_stamp);
    expect_ok(api->result_stamp(result, &retained_stamp), "result_stamp after parent teardown");
    expect(same_stamp(&frame_stamp, &retained_stamp), "the result retains its exact source frame");
    memset(&result_info, 0, sizeof(result_info));
    result_info.struct_size = (uint32_t)sizeof(result_info);
    expect_ok(api->result_describe(result, &result_info), "result_describe after parent teardown");
    expect(result_info.match_count == 2, "the frozen header still sees both planted copies");
    for (index = 0; index < result_info.match_count; index += 1) {
        madopilot_match_t match;
        size_t planted;
        memset(&match, 0, sizeof(match));
        match.struct_size = (uint32_t)sizeof(match);
        if (!expect_ok(api->result_match(result, index, &match), "result_match")) {
            continue;
        }
        expect(match.score > 0.999 && match.score < 1.001 &&
                   equals(match.template_id, "panel.patch"),
               "a planted copy retains its template identity and unit correlation");
        for (planted = 0; planted < 2; planted += 1) {
            if (match.bounds.space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
                match.bounds.left == (int32_t)SCENE_PLANTED[planted][0] &&
                match.bounds.top == (int32_t)SCENE_PLANTED[planted][1] &&
                match.bounds.right == (int32_t)(SCENE_PLANTED[planted][0] + PATCH_WIDTH) &&
                match.bounds.bottom == (int32_t)(SCENE_PLANTED[planted][1] + PATCH_HEIGHT)) {
                found[planted] = 1;
            }
        }
    }
    expect(found[0] && found[1], "both planted rectangles retain their exact capture-pixel bounds");
    memset(&image, 0, sizeof(image));
    image.struct_size = (uint32_t)sizeof(image);
    if (expect_ok(api->mapping_describe(mapping, &image), "mapping_describe after parent teardown")) {
        expect(image.width == SCENE_WIDTH && image.height == SCENE_HEIGHT &&
                   image.format == MADOPILOT_PIXEL_FORMAT_RGBA8 && image.bytes.len == SCENE_BYTES,
               "retained mapping preserves the released image layout");
        if (expect(image.bytes.data != NULL && image.bytes.len >= 4,
                   "retained mapping exposes readable pixels")) {
            scene_pixel(0, 0, pixel);
            expect(image.bytes.data[0] == pixel[0] && image.bytes.data[1] == pixel[1] &&
                       image.bytes.data[2] == pixel[2] && image.bytes.data[3] == 255,
                   "mapped pixels survive source and parent release");
        }
    }
    memset(&retained_stamp, 0, sizeof(retained_stamp));
    retained_stamp.struct_size = (uint32_t)sizeof(retained_stamp);
    expect_ok(api->mapping_stamp(mapping, &retained_stamp), "mapping_stamp after parent teardown");
    expect(same_stamp(&frame_stamp, &retained_stamp), "mapping and result retain the same complete stamp");
    completed = 1;

cleanup:
    api->error_release(error);
    api->result_release(result);
    api->template_release(prepared);
    api->package_release(package);
    api->mapping_release(mapping);
    api->frame_release(frame);
    api->session_release(session);
    api->target_list_release(targets);
    api->engine_release(engine);
    free(scene);
    return completed;
}

int main(int argc, char** argv)
{
    const madopilot_api_t* api = NULL;
    const char* package_dir = NULL;
    int index;
    for (index = 1; index < argc; index += 1) {
        if (strcmp(argv[index], "--package") == 0 && index + 1 < argc) {
            package_dir = argv[++index];
        } else {
            fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
            return 2;
        }
    }
    if (package_dir == NULL) {
        fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
        return 2;
    }
    printf("frozen header: abi %u.%u, table %u bytes as v1_5 declared it\n",
           (unsigned)MADOPILOT_ABI_MAJOR, (unsigned)MADOPILOT_ABI_MINOR,
           (unsigned)sizeof(madopilot_api_t));
    if (!negotiate_at_the_mandatory_prefix()) {
        return 1;
    }
    negotiation_refusals();
    if (!expect_ok(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                                     sizeof(madopilot_api_t), &api),
                   "negotiating at the frozen header's full size") ||
        !expect(api != NULL && api->struct_size >= sizeof(madopilot_api_t),
                "the complete ABI 1.5 table is available")) {
        return 1;
    }
    printf("library table: %u bytes, abi %u.%u\n", (unsigned)api->struct_size,
           (unsigned)api->abi_major, (unsigned)api->abi_minor);
    if (!run_the_flow(api, package_dir)) {
        return 1;
    }
    if (failures != 0) {
        printf("madopilot-abi-compat-v1_5 failed %d check(s)\n", failures);
        return 1;
    }
    printf("madopilot-abi-compat-v1_5 complete\n");
    return 0;
}
