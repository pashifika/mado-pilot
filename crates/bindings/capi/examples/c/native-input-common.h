/*
 * Shared implementation for the two platform-native C examples.
 *
 * The including source defines only the platform policy: native source kind,
 * required input pairs, delivery mechanism, focus policy, and permission
 * behavior. Keeping the flow here makes the safety checks identical on both
 * targets while each public example remains explicit about what its platform
 * can do.
 *
 * This is example source, not a public header.
 */
#ifndef MADOPILOT_NATIVE_INPUT_COMMON_H
#define MADOPILOT_NATIVE_INPUT_COMMON_H

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <psapi.h>
#pragma comment(lib, "psapi.lib")
#elif defined(__APPLE__)
#include <sys/resource.h>
#include <time.h>
#endif


#include "madopilot/madopilot.h"
#include "../native-expected-condition.h"

#ifndef MADOPILOT_EXAMPLE_NAME
#error "the platform example must define MADOPILOT_EXAMPLE_NAME"
#endif
#ifndef MADOPILOT_EXAMPLE_SOURCE_KIND
#error "the platform example must define MADOPILOT_EXAMPLE_SOURCE_KIND"
#endif
#ifndef MADOPILOT_EXAMPLE_REQUIRED_PAIRS
#error "the platform example must define MADOPILOT_EXAMPLE_REQUIRED_PAIRS"
#endif
#ifndef MADOPILOT_EXAMPLE_DELIVERY
#error "the platform example must define MADOPILOT_EXAMPLE_DELIVERY"
#endif
#ifndef MADOPILOT_EXAMPLE_FOCUS
#error "the platform example must define MADOPILOT_EXAMPLE_FOCUS"
#endif
#ifndef MADOPILOT_EXAMPLE_READS_PERMISSIONS
#error "the platform example must define MADOPILOT_EXAMPLE_READS_PERMISSIONS"
#endif
#ifndef MADOPILOT_EXAMPLE_ALLOWS_UNKNOWN
#error "the platform example must define MADOPILOT_EXAMPLE_ALLOWS_UNKNOWN"
#endif

static int failures = 0;

static const uint64_t diagnostic_activity_tag = UINT64_C(0x4d50494e505554a1);
static const uint32_t diagnostic_capacity = UINT32_C(256);

static int expect(int condition, const char* what)
{
    if (!condition) {
        fprintf(stderr, "FAIL: %s\n", what);
        failures += 1;
    }
    return condition;
}

static int require_ok(const madopilot_api_t* api,
                      madopilot_status_t status,
                      madopilot_error_t** error,
                      const char* what)
{
    if (status == MADOPILOT_STATUS_OK) {
        if (*error != NULL) {
            api->error_release(*error);
            *error = NULL;
            return expect(0, "a successful call returned an error handle");
        }
        return 1;
    }

    fprintf(stderr, "%s: status %d", what, (int)status);
    if (*error != NULL) {
        madopilot_error_detail_t detail;
        memset(&detail, 0, sizeof(detail));
        detail.struct_size = (uint32_t)sizeof(detail);
        if (api->error_describe(*error, &detail) == MADOPILOT_STATUS_OK) {
            fprintf(stderr, " category %d", (int)detail.category);
        }
        api->error_release(*error);
        *error = NULL;
    }
    fputc('\n', stderr);
    failures += 1;
    return 0;
}

static int bounded_operation(const madopilot_api_t* api,
                             uint64_t budget_nanos,
                             madopilot_operation_t* operation)
{
    uint64_t now = 0;
    if (!expect(api->clock_now(&now) == MADOPILOT_STATUS_OK,
                "clock_now creates an absolute deadline")) {
        return 0;
    }

    memset(operation, 0, sizeof(*operation));
    operation->struct_size = (uint32_t)sizeof(*operation);
    operation->flags = MADOPILOT_OPERATION_HAS_DEADLINE |
                       MADOPILOT_OPERATION_HAS_ACTIVITY_TAG;
    operation->deadline_nanos = now + budget_nanos;
    operation->activity_tag = diagnostic_activity_tag;
    return 1;
}

static int drain_diagnostics(const madopilot_api_t* api,
                             madopilot_diagnostic_reader_t* reader,
                             int require_mapping)
{
    uint64_t records = 0;
    uint64_t normal = 0;
    uint64_t debug = 0;
    uint64_t mappings = 0;
    uint64_t discarded_normal = 0;
    uint64_t discarded_debug = 0;

    for (;;) {
        madopilot_diagnostic_drain_state_t state =
            MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY;
        madopilot_diagnostic_batch_t* batch = NULL;
        madopilot_diagnostic_batch_info_t info;
        uint64_t index;

        if (!expect(api->diagnostic_reader_drain(reader, &state, &batch) ==
                        MADOPILOT_STATUS_OK,
                    "diagnostic_reader_drain")) {
            return 0;
        }
        if (state == MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM) {
            if (!expect(batch == NULL, "end-of-stream has no diagnostic batch")) {
                return 0;
            }
            break;
        }
        if (!expect(state == MADOPILOT_DIAGNOSTIC_DRAIN_BATCH && batch != NULL,
                    "a sealed diagnostic reader yields batches then ends")) {
            if (batch != NULL) {
                api->diagnostic_batch_release(batch);
            }
            return 0;
        }

        memset(&info, 0, sizeof(info));
        info.struct_size = (uint32_t)sizeof(info);
        if (!expect(api->diagnostic_batch_info(batch, &info) ==
                        MADOPILOT_STATUS_OK,
                    "diagnostic_batch_info")) {
            api->diagnostic_batch_release(batch);
            return 0;
        }
        discarded_normal += info.discarded_normal;
        discarded_debug += info.discarded_debug;
        for (index = 0; index < info.record_count; ++index) {
            madopilot_diagnostic_record_t record;
            memset(&record, 0, sizeof(record));
            record.struct_size = (uint32_t)sizeof(record);
            if (!expect(api->diagnostic_batch_record_at(
                            batch, (size_t)index, &record) ==
                            MADOPILOT_STATUS_OK,
                        "diagnostic_batch_record_at") ||
                !expect((record.flags &
                         MADOPILOT_DIAGNOSTIC_RECORD_HAS_ACTIVITY) != 0 &&
                            record.activity_tag == diagnostic_activity_tag,
                        "every diagnostic record retains the caller activity")) {
                api->diagnostic_batch_release(batch);
                return 0;
            }
            records += 1;
            normal += record.level == MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL;
            debug += record.level == MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG;
            if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_MAPPING) {
                const uint32_t required =
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET |
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME |
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE |
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE;
                if (!expect((record.flags & required) == required &&
                                record.source_space ==
                                    MADOPILOT_SPACE_CAPTURE_PIXELS &&
                                record.destination_space ==
                                    MADOPILOT_SPACE_CAPTURE_PIXELS,
                            "mapping diagnostics expose copied identity and spaces")) {
                    api->diagnostic_batch_release(batch);
                    return 0;
                }
                mappings += 1;
            }
        }
        api->diagnostic_batch_release(batch);
    }

    printf("diagnostics: %llu record(s), normal %llu, debug %llu, "
           "discarded-normal %llu, discarded-debug %llu\n",
           (unsigned long long)records, (unsigned long long)normal,
           (unsigned long long)debug, (unsigned long long)discarded_normal,
           (unsigned long long)discarded_debug);
    return expect(records != 0, "the enabled diagnostic stream retained records") &&
           (!require_mapping ||
            expect(mappings != 0, "the mapped frame emitted a debug mapping fact"));
}

static int same_text(madopilot_str_t view, const char* text)
{
    const size_t length = strlen(text);
    return view.len == length &&
           (length == 0 || (view.data != NULL && memcmp(view.data, text, length) == 0));
}

static void pause_for_new_frame(void)
{
#if defined(_WIN32)
    Sleep(10u);
#elif defined(__APPLE__)
    const struct timespec delay = { 0, 10000000 };
    (void)nanosleep(&delay, NULL);
#endif
}

static int same_frame_stamp(const madopilot_frame_stamp_t* left,
                            const madopilot_frame_stamp_t* right)
{
    return left->stream == right->stream &&
           left->epoch == right->epoch &&
           left->sequence == right->sequence &&
           left->geometry == right->geometry;
}

static int strictly_newer_frame(const madopilot_frame_stamp_t* candidate,
                                const madopilot_frame_stamp_t* before)
{
    return candidate->stream == before->stream &&
           (candidate->epoch > before->epoch ||
            (candidate->epoch == before->epoch &&
             candidate->sequence > before->sequence));
}

static int observe_expected_condition(const madopilot_api_t* api,
                                      madopilot_session_t* session,
                                      const madopilot_frame_stamp_t* before)
{
    madopilot_operation_t operation;

    if (!bounded_operation(api, UINT64_C(5000000000), &operation)) {
        return 0;
    }
    for (;;) {
        madopilot_frame_t* frame = NULL;
        madopilot_mapping_t* mapping = NULL;
        madopilot_frame_stamp_t frame_stamp;
        madopilot_frame_stamp_t mapping_stamp;
        madopilot_map_request_t map_request;
        madopilot_image_t image;
        madopilot_error_t* error = NULL;
        int matched;

        if (!require_ok(api,
                        api->session_acquire_frame(
                            session, &operation, &frame, &error),
                        &error, "observe a newer frame")) {
            return 0;
        }
        memset(&frame_stamp, 0, sizeof(frame_stamp));
        frame_stamp.struct_size = (uint32_t)sizeof(frame_stamp);
        if (!expect(api->frame_stamp(frame, &frame_stamp) == MADOPILOT_STATUS_OK,
                    "read the observed frame identity")) {
            api->frame_release(frame);
            return 0;
        }
        if (!expect(frame_stamp.stream == before->stream,
                    "the observation remains correlated to the source stream")) {
            api->frame_release(frame);
            return 0;
        }
        if (!strictly_newer_frame(&frame_stamp, before)) {
            api->frame_release(frame);
            pause_for_new_frame();
            continue;
        }

        memset(&map_request, 0, sizeof(map_request));
        map_request.struct_size = (uint32_t)sizeof(map_request);
        map_request.format = MADOPILOT_PIXEL_FORMAT_BGRA8;
        if (!require_ok(api,
                        api->frame_map(
                            frame, &map_request, &operation, &mapping, &error),
                        &error, "map an observed frame")) {
            api->frame_release(frame);
            return 0;
        }
        memset(&mapping_stamp, 0, sizeof(mapping_stamp));
        mapping_stamp.struct_size = (uint32_t)sizeof(mapping_stamp);
        memset(&image, 0, sizeof(image));
        image.struct_size = (uint32_t)sizeof(image);
        if (!expect(api->mapping_stamp(mapping, &mapping_stamp) ==
                        MADOPILOT_STATUS_OK,
                    "read the observed mapping identity") ||
            !expect(same_frame_stamp(&mapping_stamp, &frame_stamp),
                    "the visual search remains correlated to the observed frame") ||
            !expect(api->mapping_describe(mapping, &image) ==
                        MADOPILOT_STATUS_OK,
                    "describe the observed mapping")) {
            api->mapping_release(mapping);
            api->frame_release(frame);
            return 0;
        }
        matched = image.format == MADOPILOT_PIXEL_FORMAT_BGRA8 &&
                  madopilot_example_expected_condition_matches(
                      image.bytes.data, image.bytes.len, image.stride,
                      image.width, image.height);
        api->mapping_release(mapping);
        api->frame_release(frame);
        if (matched) {
            printf("expected condition: stream %llu epoch %llu sequence %llu\n",
                   (unsigned long long)frame_stamp.stream,
                   (unsigned long long)frame_stamp.epoch,
                   (unsigned long long)frame_stamp.sequence);
            return 1;
        }
        pause_for_new_frame();
    }
}

static int probe_permissions(const madopilot_api_t* api,
                             madopilot_engine_t* engine,
                             const madopilot_operation_t* operation,
                             int require_granted)
{
    const madopilot_permission_kind_t kinds[2] = {
        MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE,
        MADOPILOT_PERMISSION_KIND_INPUT_CONTROL,
    };
    size_t index;

    for (index = 0; index < 2; index += 1) {
        madopilot_permission_t permission;
        madopilot_error_t* error = NULL;
        madopilot_status_t status;

        memset(&permission, 0, sizeof(permission));
        permission.struct_size = (uint32_t)sizeof(permission);
        status = api->engine_permission(
            engine, kinds[index], operation, &permission, &error);
        if (MADOPILOT_EXAMPLE_READS_PERMISSIONS) {
            if (!require_ok(api, status, &error, "engine_permission")) {
                return 0;
            }
            printf("permission kind %d state %d\n",
                   (int)permission.kind, (int)permission.state);
            if (require_granted &&
                !expect(permission.state == MADOPILOT_PERMISSION_STATE_GRANTED,
                        "the required native permission is not granted")) {
                return 0;
            }
        } else {
            if (!expect(status == MADOPILOT_STATUS_UNSUPPORTED,
                        "this platform reports that it reads no permission state")) {
                if (error != NULL) {
                    api->error_release(error);
                }
                return 0;
            }
            expect(error != NULL,
                   "an unavailable permission probe returns owned detail");
            api->error_release(error);
        }
    }

    return 1;
}

static int select_target(const madopilot_api_t* api,
                         madopilot_target_list_t* targets,
                         const char* title,
                         size_t* selected,
                         uint64_t* target_id,
                         madopilot_input_address_scope_t* address_scope,
                         madopilot_submission_evidence_t* evidence)
{
    const madopilot_input_operation_kind_t operations[3] = {
        MADOPILOT_INPUT_OPERATION_POINTER,
        MADOPILOT_INPUT_OPERATION_KEYBOARD,
        MADOPILOT_INPUT_OPERATION_TEXT,
    };
    const madopilot_input_delivery_t routes[3] = {
        MADOPILOT_INPUT_DELIVERY_SYSTEM,
        MADOPILOT_INPUT_DELIVERY_WINDOW_MESSAGE,
        MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED,
    };
    size_t count = 0;
    size_t index;
    size_t matches = 0;
    *address_scope = MADOPILOT_INPUT_ADDRESS_NONE;
    *evidence = MADOPILOT_SUBMISSION_EVIDENCE_NONE;

    if (!expect(api->target_list_count(targets, &count) == MADOPILOT_STATUS_OK,
                "target_list_count")) {
        return 0;
    }
    printf("discovered: %zu target(s)\n", count);

    for (index = 0; index < count; index += 1) {
        madopilot_target_t target;
        memset(&target, 0, sizeof(target));
        target.struct_size = (uint32_t)sizeof(target);
        if (!expect(api->target_list_get(targets, index, &target) == MADOPILOT_STATUS_OK,
                    "target_list_get")) {
            return 0;
        }
        if (target.kind == MADOPILOT_TARGET_KIND_WINDOW && same_text(target.name, title)) {
            *selected = index;
            *target_id = target.target;
            matches += 1;
        }
    }

    if (!expect(matches == 1,
                "the full title must identify exactly one window before input exists")) {
        return 0;
    }

    for (index = 0; index < 9; index += 1) {
        const uint64_t bit = UINT64_C(1) << index;
        madopilot_input_capability_t capability;
        if ((MADOPILOT_EXAMPLE_REQUIRED_PAIRS & bit) == 0) {
            continue;
        }
        memset(&capability, 0, sizeof(capability));
        capability.struct_size = (uint32_t)sizeof(capability);
        if (!expect(api->target_list_input_capability(
                        targets, *selected, operations[index / 3], routes[index % 3],
                        &capability) == MADOPILOT_STATUS_OK,
                    "target_list_input_capability") ||
            !expect(capability.support == MADOPILOT_CAPABILITY_SUPPORTED ||
                        (MADOPILOT_EXAMPLE_ALLOWS_UNKNOWN &&
                         capability.support == MADOPILOT_CAPABILITY_UNKNOWN),
                    "the selected target can attempt every required input pair") ||
            !expect((capability.flags & MADOPILOT_INPUT_CAPABILITY_HAS_EVIDENCE) != 0,
                    "every attemptable pair reports its strongest evidence")) {
            return 0;
        }
        if (*address_scope == MADOPILOT_INPUT_ADDRESS_NONE) {
            *address_scope = capability.address_scope;
            *evidence = capability.evidence;
        } else if (!expect(*address_scope == capability.address_scope &&
                               *evidence == capability.evidence,
                           "required pairs share one route scope and evidence")) {
            return 0;
        }
        if (capability.operation == MADOPILOT_INPUT_OPERATION_POINTER &&
            !expect((capability.pointer_spaces &
                     (UINT32_C(1) << MADOPILOT_SPACE_CAPTURE_PIXELS)) != 0,
                    "the target accepts capture-pixel pointer coordinates")) {
            return 0;
        }
    }

    printf("selected: index %zu target %llu\n", *selected,
           (unsigned long long)*target_id);
    return 1;
}

static void set_key_event(madopilot_input_event_t* event,
                          madopilot_input_event_kind_t kind,
                          uint32_t scalar)
{
    memset(event, 0, sizeof(*event));
    event->struct_size = (uint32_t)sizeof(*event);
    event->kind = kind;
    event->key = MADOPILOT_KEY_CHARACTER;
    event->key_value = scalar;
}

static int deliver(const madopilot_api_t* api,
                   madopilot_session_t* session,
                   madopilot_frame_t* frame,
                   const madopilot_frame_info_t* frame_info,
                   madopilot_input_address_scope_t address_scope,
                   madopilot_submission_evidence_t evidence)
{
    madopilot_input_event_t events[4];
    madopilot_input_delivery_t delivery = MADOPILOT_EXAMPLE_DELIVERY;
    madopilot_input_request_t request;
    madopilot_input_receipt_info_t info;
    madopilot_input_receipt_t* receipt = NULL;
    madopilot_operation_t operation;
    madopilot_error_t* error = NULL;

    memset(events, 0, sizeof(events));
    events[0].struct_size = (uint32_t)sizeof(events[0]);
    events[0].kind = MADOPILOT_INPUT_EVENT_POINTER_MOVE;
    events[0].space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    events[0].x = (double)frame_info->width / 2.0;
    events[0].y = (double)frame_info->height / 2.0;
    set_key_event(&events[1], MADOPILOT_INPUT_EVENT_KEY_PRESS, (uint32_t)'m');
    set_key_event(&events[2], MADOPILOT_INPUT_EVENT_KEY_RELEASE, (uint32_t)'m');
    events[3].struct_size = (uint32_t)sizeof(events[3]);
    events[3].kind = MADOPILOT_INPUT_EVENT_DELAY;
    events[3].delay_nanos = UINT64_C(20000000);

    memset(&request, 0, sizeof(request));
    request.struct_size = (uint32_t)sizeof(request);
    request.events = events;
    request.event_count = sizeof(events) / sizeof(events[0]);
    request.event_stride = sizeof(events[0]);
    request.deliveries = &delivery;
    request.delivery_count = 1;
    request.focus_policy = MADOPILOT_EXAMPLE_FOCUS;
    request.geometry_policy = MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED;
    request.source_frame = frame;

    if (!bounded_operation(api, UINT64_C(2000000000), &operation) ||
        !require_ok(api,
                    api->session_send_input(session, &request, &operation,
                                            &receipt, &error),
                    &error, "session_send_input")) {
        return 0;
    }
    memset(&info, 0, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    if (!expect(api->input_receipt_info(receipt, &info) == MADOPILOT_STATUS_OK,
                "input_receipt_info")) {
        api->input_receipt_release(receipt);
        return 0;
    }

    printf("receipt: outcome %d submitted %" PRIu64 " fault %d cleanup %d\n",
           (int)info.outcome, info.submitted,
           (int)info.fault, (int)info.cleanup);
    api->input_receipt_release(receipt);
    return expect(info.outcome == MADOPILOT_SEQUENCE_COMPLETE &&
                      info.submitted ==
                          (uint64_t)(sizeof(events) / sizeof(events[0])) &&
                      (info.flags & (MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE |
                                     MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE)) ==
                          (MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE |
                           MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE) &&
                      info.selected_route == MADOPILOT_EXAMPLE_DELIVERY &&
                      info.address_scope == address_scope &&
                      info.evidence == evidence,
                  "the bounded native sequence completed with truthful route evidence");
}

static int run_native(const madopilot_api_t* api, const char* title, int check_only)
{
    madopilot_source_t source;
    madopilot_engine_options_t engine_options;
    madopilot_engine_capabilities_t engine_capabilities;
    madopilot_operation_t operation;
    madopilot_open_request_t open_request;
    madopilot_input_open_request_t input_open_request;
    madopilot_input_descriptor_t input_descriptor;
    madopilot_session_info_t session_info;
    madopilot_frame_info_t frame_info;
    madopilot_frame_stamp_t frame_stamp;
    madopilot_map_request_t map_request;
    madopilot_image_t image;
    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_mapping_t* mapping = NULL;
    madopilot_error_t* error = NULL;
    madopilot_diagnostic_reader_t* diagnostic_reader = NULL;
    uint64_t target_id = 0;
    size_t selected = 0;
    madopilot_input_address_scope_t address_scope = MADOPILOT_INPUT_ADDRESS_NONE;
    madopilot_submission_evidence_t evidence = MADOPILOT_SUBMISSION_EVIDENCE_NONE;
    int worked = 0;

    memset(&source, 0, sizeof(source));
    source.struct_size = (uint32_t)sizeof(source);
    source.kind = MADOPILOT_EXAMPLE_SOURCE_KIND;
    memset(&engine_options, 0, sizeof(engine_options));
    engine_options.struct_size = (uint32_t)sizeof(engine_options);
    engine_options.diagnostic_level = MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG;
    engine_options.diagnostic_capacity = diagnostic_capacity;
    if (!bounded_operation(api, UINT64_C(5000000000), &operation) ||
        !require_ok(api,
                    api->engine_create_with_options(
                        &source, &engine_options, &operation, &engine, &error),
                    &error, "engine_create_with_options")) {
        goto cleanup;
    }
    if (!expect(api->engine_take_diagnostic_reader(
                    engine, &diagnostic_reader) == MADOPILOT_STATUS_OK &&
                    diagnostic_reader != NULL,
                "an enabled engine exposes one diagnostic reader")) {
        goto cleanup;
    }

    memset(&engine_capabilities, 0, sizeof(engine_capabilities));
    engine_capabilities.struct_size = (uint32_t)sizeof(engine_capabilities);
    if (!expect(api->engine_capabilities(engine, &engine_capabilities) ==
                    MADOPILOT_STATUS_OK,
                "engine_capabilities") ||
        !expect((engine_capabilities.flags & MADOPILOT_ENGINE_DELIVERS_INPUT) != 0,
                "the native engine exposes input capability") ||
        !expect(((engine_capabilities.flags & MADOPILOT_ENGINE_READS_PERMISSIONS) != 0) ==
                    (MADOPILOT_EXAMPLE_READS_PERMISSIONS != 0),
                "the native engine reports its permission behavior")) {
        goto cleanup;
    }
    printf("engine capabilities: 0x%x\n", (unsigned)engine_capabilities.flags);

    if (!probe_permissions(api, engine, &operation, !check_only)) {
        goto cleanup;
    }
    if (check_only) {
        worked = 1;
        goto cleanup;
    }

    if (!bounded_operation(api, UINT64_C(5000000000), &operation) ||
        !require_ok(api, api->engine_discover(engine, &operation, &targets, &error),
                    &error, "engine_discover") ||
        !select_target(api, targets, title, &selected, &target_id, &address_scope,
                       &evidence)) {
        goto cleanup;
    }

    memset(&input_descriptor, 0, sizeof(input_descriptor));
    input_descriptor.struct_size = (uint32_t)sizeof(input_descriptor);
    if (!require_ok(api,
                    api->engine_input_descriptor(engine, targets, selected, &operation,
                                                 &input_descriptor, &error),
                    &error, "engine_input_descriptor") ||
        !expect((input_descriptor.known_pairs &
                 (input_descriptor.supported_pairs |
                  (MADOPILOT_EXAMPLE_ALLOWS_UNKNOWN
                       ? input_descriptor.unknown_pairs
                       : UINT64_C(0))) &
                 MADOPILOT_EXAMPLE_REQUIRED_PAIRS) ==
                    MADOPILOT_EXAMPLE_REQUIRED_PAIRS,
                "the live descriptor retains every attemptable required pair")) {
        goto cleanup;
    }

    memset(&open_request, 0, sizeof(open_request));
    open_request.struct_size = (uint32_t)sizeof(open_request);
    memset(&input_open_request, 0, sizeof(input_open_request));
    input_open_request.struct_size = (uint32_t)sizeof(input_open_request);
    input_open_request.requirement = MADOPILOT_INPUT_REQUIRED;
    input_open_request.required_pairs = MADOPILOT_EXAMPLE_REQUIRED_PAIRS;
    if (!bounded_operation(api, UINT64_C(5000000000), &operation) ||
        !require_ok(api,
                    api->session_open_with_input(engine, targets, selected,
                                                 &open_request, &input_open_request,
                                                 &operation, &session, &error),
                    &error, "session_open_with_input")) {
        goto cleanup;
    }
    api->target_list_release(targets);
    targets = NULL;

    memset(&session_info, 0, sizeof(session_info));
    session_info.struct_size = (uint32_t)sizeof(session_info);
    if (!expect(api->session_describe(session, &session_info) == MADOPILOT_STATUS_OK,
                "session_describe") ||
        !expect(session_info.accepts_input == 1,
                "the opened session established input")) {
        goto cleanup;
    }
    printf("session: stream %llu target %llu input %d\n",
           (unsigned long long)session_info.stream,
           (unsigned long long)session_info.target,
           (int)session_info.accepts_input);

    if (!bounded_operation(api, UINT64_C(5000000000), &operation) ||
        !require_ok(api,
                    api->session_acquire_frame(session, &operation, &frame, &error),
                    &error, "session_acquire_frame")) {
        goto cleanup;
    }
    memset(&frame_info, 0, sizeof(frame_info));
    frame_info.struct_size = (uint32_t)sizeof(frame_info);
    memset(&frame_stamp, 0, sizeof(frame_stamp));
    frame_stamp.struct_size = (uint32_t)sizeof(frame_stamp);
    if (!expect(api->frame_describe(frame, &frame_info) == MADOPILOT_STATUS_OK,
                "frame_describe") ||
        !expect(api->frame_stamp(frame, &frame_stamp) == MADOPILOT_STATUS_OK,
                "frame_stamp")) {
        goto cleanup;
    }
    printf("frame: epoch %llu sequence %llu geometry %llu %ux%u\n",
           (unsigned long long)frame_stamp.epoch,
           (unsigned long long)frame_stamp.sequence,
           (unsigned long long)frame_stamp.geometry,
           (unsigned)frame_info.width, (unsigned)frame_info.height);

    memset(&map_request, 0, sizeof(map_request));
    map_request.struct_size = (uint32_t)sizeof(map_request);
    map_request.format = MADOPILOT_PIXEL_FORMAT_BGRA8;
    if (!bounded_operation(api, UINT64_C(5000000000), &operation) ||
        !require_ok(api,
                    api->frame_map(frame, &map_request, &operation, &mapping, &error),
                    &error, "frame_map")) {
        goto cleanup;
    }
    memset(&image, 0, sizeof(image));
    image.struct_size = (uint32_t)sizeof(image);
    if (!expect(api->mapping_describe(mapping, &image) == MADOPILOT_STATUS_OK,
                "mapping_describe") ||
        !expect(image.format == MADOPILOT_PIXEL_FORMAT_BGRA8 &&
                    !madopilot_example_expected_condition_matches(
                        image.bytes.data, image.bytes.len, image.stride,
                        image.width, image.height),
                "the expected visual condition is absent before input")) {
        goto cleanup;
    }
    printf("mapping: %zu byte(s)\n", image.bytes.len);

    worked = deliver(api, session, frame, &frame_info, address_scope, evidence);
    if (worked) {
        api->mapping_release(mapping);
        mapping = NULL;
        api->frame_release(frame);
        frame = NULL;
        worked = observe_expected_condition(api, session, &frame_stamp);
    }

cleanup:
    if (mapping != NULL) {
        api->mapping_release(mapping);
    }
    if (frame != NULL) {
        api->frame_release(frame);
    }
    if (session != NULL) {
        if (bounded_operation(api, UINT64_C(2000000000), &operation)) {
            require_ok(api, api->session_close(session, &operation, &error),
                       &error, "session_close");
        }
        api->session_release(session);
    }
    if (targets != NULL) {
        api->target_list_release(targets);
    }
    if (engine != NULL) {
        api->engine_release(engine);
        engine = NULL;
    }
    if (diagnostic_reader != NULL) {
        (void)drain_diagnostics(api, diagnostic_reader, !check_only);
        expect(api->diagnostic_reader_release(diagnostic_reader) ==
                   MADOPILOT_STATUS_OK,
               "diagnostic_reader_release");
    }
    if (error != NULL) {
        api->error_release(error);
    }

    return worked && failures == 0;
}
static uint64_t peak_resident_bytes(void)
{
#if defined(_WIN32)
    PROCESS_MEMORY_COUNTERS counters;
    memset(&counters, 0, sizeof(counters));
    counters.cb = sizeof(counters);
    if (!GetProcessMemoryInfo(GetCurrentProcess(), &counters, sizeof(counters))) {
        return 0;
    }
    return (uint64_t)counters.PeakWorkingSetSize;
#elif defined(__APPLE__)
    struct rusage usage;
    memset(&usage, 0, sizeof(usage));
    if (getrusage(RUSAGE_SELF, &usage) != 0) {
        return 0;
    }
    return (uint64_t)usage.ru_maxrss;
#else
    return 0;
#endif
}

static void report_peak_resident_bytes(void)
{
    printf("peak resident: %llu byte(s)\n",
           (unsigned long long)peak_resident_bytes());
}


int main(int argc, char** argv)
{
    const madopilot_api_t* api = NULL;
    const madopilot_api_t* rejected_api =
        (const madopilot_api_t*)(uintptr_t)1;
    const char* title = NULL;
    int check_only = 0;
    int load_only = 0;

    if (argc == 2 && strcmp(argv[1], "--load-check") == 0) {
        load_only = 1;
    } else if (argc == 2 && strcmp(argv[1], "--check") == 0) {
        check_only = 1;
    } else if (argc == 2 && argv[1][0] != '\0') {
        title = argv[1];
    } else {
        fprintf(stderr,
                "usage: %s --load-check | --check | \"<full fixture window title>\"\n",
                argv[0]);
        return 2;
    }

    if (!expect(madopilot_get_api(MADOPILOT_ABI_MAJOR, 1,
                                  sizeof(madopilot_api_t), &rejected_api) ==
                    MADOPILOT_STATUS_UNSUPPORTED &&
                    rejected_api == NULL,
                "reject the unreleased ABI 1.1 profile")) {
        return 1;
    }
    rejected_api = (const madopilot_api_t*)(uintptr_t)1;
    if (!expect(madopilot_get_api(MADOPILOT_ABI_MAJOR, 0,
                                  MADOPILOT_API_SIZE_ABI_1_0 + 1u,
                                  &rejected_api) ==
                    MADOPILOT_STATUS_UNSUPPORTED &&
                    rejected_api == NULL,
                "reject an ABI 1.0 caller claiming suffix entries")) {
        return 1;
    }
    if (!expect(madopilot_get_api(MADOPILOT_ABI_MAJOR, 2,
                                  sizeof(madopilot_api_t), &api) ==
                    MADOPILOT_STATUS_OK &&
                    api != NULL,
                "negotiate ABI 1.2") ||
        !expect(api->struct_size >= MADOPILOT_API_SIZE_SESSION_SEND_INPUT,
                "the negotiated table contains the ABI 1.2 input suffix")) {
        return 1;
    }

    printf("%s: abi %u.%u table %u\n", MADOPILOT_EXAMPLE_NAME,
           (unsigned)api->abi_major, (unsigned)api->abi_minor,
           (unsigned)api->struct_size);
    if (load_only) {
        report_peak_resident_bytes();
        printf("%s complete (load check)\n", MADOPILOT_EXAMPLE_NAME);
        return 0;
    }
    if (!run_native(api, title, check_only)) {
        return 1;
    }
    report_peak_resident_bytes();

    printf("%s complete%s\n", MADOPILOT_EXAMPLE_NAME,
           check_only ? " (non-prompting check)" : "");
    return 0;
}

#endif /* MADOPILOT_NATIVE_INPUT_COMMON_H */
