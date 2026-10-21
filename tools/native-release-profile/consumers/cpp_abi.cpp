/* Direct C ABI consumer compiled as C++17. Matching, CPU OCR, interruption and
 * retained results use the released table with explicit status checks and cleanup.
 */

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <string>
#include <string_view>
#include <system_error>
#include <vector>

#include "deterministic-scene.h"
#include "madopilot/madopilot.h"

namespace {

constexpr std::uint32_t blank_extent = 64;
constexpr std::size_t blank_bytes =
    static_cast<std::size_t>(blank_extent) * blank_extent * 4u;

const madopilot_api_t* api = nullptr;
const char* failed_check = nullptr;
madopilot_status_t failed_status = MADOPILOT_STATUS_OK;

bool fail(const char* check, madopilot_status_t status)
{
    if (failed_check == nullptr) {
        failed_check = check;
        failed_status = status;
    }
    return false;
}

bool require(bool condition, const char* check)
{
    return condition || fail(check, MADOPILOT_STATUS_OK);
}

bool require_ok(madopilot_status_t status, const char* check)
{
    return status == MADOPILOT_STATUS_OK || fail(check, status);
}

madopilot_str_t borrow(const char* text)
{
    return {text, std::strlen(text)};
}

bool same_text(madopilot_str_t left, const char* right)
{
    return left.data != nullptr && left.len == std::strlen(right) &&
           std::memcmp(left.data, right, left.len) == 0;
}

bool same_stamp(const madopilot_frame_stamp_t& left, const madopilot_frame_stamp_t& right)
{
    return left.stream == right.stream && left.epoch == right.epoch &&
           left.sequence == right.sequence && left.geometry == right.geometry;
}

bool same_rect(const madopilot_pixel_rect_t& rect, std::int32_t left, std::int32_t top,
               std::int32_t right, std::int32_t bottom)
{
    return rect.space == MADOPILOT_SPACE_CAPTURE_PIXELS && rect.left == left && rect.top == top &&
           rect.right == right && rect.bottom == bottom;
}

bool first_publication(const madopilot_frame_stamp_t& stamp)
{
    return stamp.stream != 0 && stamp.epoch == 0 && stamp.sequence == 0 && stamp.geometry == 0;
}

/* Refusals publish no result and return an owned error carrying the same status. */
bool refused(madopilot_status_t status, madopilot_error_t** error, const void* output,
             madopilot_status_t expected, const char* check)
{
    madopilot_error_detail_t detail{};
    detail.struct_size = static_cast<std::uint32_t>(sizeof(detail));
    const bool typed = *error != nullptr &&
        require_ok(api->error_describe(*error, &detail), check) && detail.status == status;
    const bool released = require_ok(api->error_release(*error), check);
    *error = nullptr;
    return require(status == expected && output == nullptr && typed, check) && released;
}

/* Canonical absolute UTF-8 prerequisites, including Windows extended paths. */
std::string controlled_path(const char* value)
{
    std::error_code error;
    std::string path = std::filesystem::canonical(std::filesystem::u8path(value), error).u8string();
    if (error) {
        return {};
    }
#ifdef _WIN32
    if (path.rfind(R"(\\?\)", 0) != 0) {
        path = path.rfind(R"(\\)", 0) == 0 ? R"(\\?\UNC\)" + path.substr(2) : R"(\\?\)" + path;
    }
#endif
    return path;
}

bool open_only_target(const madopilot_engine_t* engine, const madopilot_operation_t* operation,
                      const madopilot_open_request_t* request, madopilot_session_t** session,
                      const char* discover_check, const char* open_check)
{
    madopilot_target_list_t* targets = nullptr;
    std::size_t count = 0;
    const bool opened =
        require_ok(api->engine_discover(engine, operation, &targets, nullptr), discover_check) &&
        require_ok(api->target_list_count(targets, &count), discover_check) &&
        require(count == 1, discover_check) &&
        require_ok(api->session_open(engine, targets, 0, request, operation, session, nullptr),
                   open_check);
    const bool released = require_ok(api->target_list_release(targets), open_check);
    return opened && released;
}

/* Both planted copies, with exact bounds and source identity. */
bool planted_found(const madopilot_result_t* result, const madopilot_frame_stamp_t& stamp)
{
    madopilot_result_info_t info{};
    madopilot_frame_stamp_t searched{};
    info.struct_size = static_cast<std::uint32_t>(sizeof(info));
    searched.struct_size = static_cast<std::uint32_t>(sizeof(searched));
    if (!require_ok(api->result_describe(result, &info), "find-present") ||
        !require_ok(api->result_stamp(result, &searched), "find-present") ||
        info.match_count != 2 || !same_stamp(searched, stamp) ||
        !same_rect(info.searched, 0, 0, static_cast<std::int32_t>(SCENE_WIDTH),
                   static_cast<std::int32_t>(SCENE_HEIGHT))) {
        return false;
    }
    bool seen[2] = {false, false};
    for (std::size_t at = 0; at < 2; ++at) {
        madopilot_match_t match{};
        match.struct_size = static_cast<std::uint32_t>(sizeof(match));
        if (!require_ok(api->result_match(result, at, &match), "find-present") ||
            !same_text(match.template_id, "panel.patch") ||
            match.bounds.space != MADOPILOT_SPACE_CAPTURE_PIXELS ||
            match.bounds.right - match.bounds.left != static_cast<std::int32_t>(PATCH_WIDTH) ||
            match.bounds.bottom - match.bounds.top != static_cast<std::int32_t>(PATCH_HEIGHT)) {
            return false;
        }
        for (std::size_t planted = 0; planted < 2; ++planted) {
            if (match.bounds.left == static_cast<std::int32_t>(SCENE_PLANTED[planted][0]) &&
                match.bounds.top == static_cast<std::int32_t>(SCENE_PLANTED[planted][1])) {
                if (seen[planted]) {
                    return false;
                }
                seen[planted] = true;
            }
        }
    }
    return seen[0] && seen[1];
}

bool nothing_found(const madopilot_result_t* result, const madopilot_frame_stamp_t& stamp)
{
    madopilot_result_info_t info{};
    madopilot_frame_stamp_t searched{};
    info.struct_size = static_cast<std::uint32_t>(sizeof(info));
    searched.struct_size = static_cast<std::uint32_t>(sizeof(searched));
    return require_ok(api->result_describe(result, &info), "find-absent") &&
           require_ok(api->result_stamp(result, &searched), "find-absent") && info.match_count == 0 &&
           same_stamp(searched, stamp) &&
           same_rect(info.searched, 0, 0, static_cast<std::int32_t>(SCENE_WIDTH),
                     static_cast<std::int32_t>(SCENE_HEIGHT));
}

bool mapping_readable(const madopilot_mapping_t* mapping, const madopilot_frame_stamp_t& stamp,
                      const std::vector<std::uint8_t>& scene)
{
    madopilot_image_t image{};
    madopilot_frame_stamp_t mapped{};
    image.struct_size = static_cast<std::uint32_t>(sizeof(image));
    mapped.struct_size = static_cast<std::uint32_t>(sizeof(mapped));
    return require_ok(api->mapping_describe(mapping, &image), "matching-map") &&
           require_ok(api->mapping_stamp(mapping, &mapped), "matching-map") &&
           image.width == SCENE_WIDTH && image.height == SCENE_HEIGHT &&
           image.bytes.data != nullptr && image.bytes.len == scene.size() &&
           std::memcmp(image.bytes.data, scene.data(), scene.size()) == 0 && same_stamp(mapped, stamp);
}

bool empty_recognition(const madopilot_ocr_result_t* result, const madopilot_frame_stamp_t& stamp,
                       std::int32_t left, std::int32_t top,
                       std::int32_t right, std::int32_t bottom)
{
    madopilot_ocr_result_info_t info{};
    info.struct_size = static_cast<std::uint32_t>(sizeof(info));
    return require_ok(api->ocr_result_info(result, &info), "ocr-result") && info.region_count == 0 &&
           same_stamp(info.source, stamp) &&
           same_rect(info.effective_region, left, top, right, bottom) &&
           info.output_space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
           same_text(info.backend_id, "onnxruntime-cpu") &&
           same_text(info.backend_version, "0.5.0+ort-1.29.0-api17") &&
           same_text(info.model_id, "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1") &&
           same_text(info.model_version, "rapidocr-3.9.2+095232a4c94f7f0e6600ba5bba1177010ad696d4") &&
           same_text(info.profile_id, "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1");
}

bool cancelled_operation(madopilot_operation_t& operation,
                         madopilot_cancellation_t** token, const char* check)
{
    std::int32_t cancelled = 0;
    operation = {};
    operation.struct_size = static_cast<std::uint32_t>(sizeof(operation));
    if (!require_ok(api->cancellation_create(token), check) ||
        !require_ok(api->cancellation_cancel(*token), check) ||
        !require_ok(api->cancellation_is_cancelled(*token, &cancelled), check) ||
        !require(cancelled != 0, check)) {
        return false;
    }
    operation.cancellation = *token;
    return true;
}

bool expired_operation(madopilot_operation_t& operation, const char* check)
{
    operation = {};
    operation.struct_size = static_cast<std::uint32_t>(sizeof(operation));
    operation.flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    return require_ok(api->clock_now(&operation.deadline_nanos), check);
}

bool closed_idempotently(const madopilot_session_t* session,
                         const madopilot_operation_t* operation, const char* check)
{
    std::int32_t closed = 0;
    if (!require_ok(api->session_close(session, operation, nullptr), check) ||
        !require_ok(api->session_close(session, operation, nullptr), check) ||
        !require_ok(api->session_is_closed(session, &closed), check) ||
        !require(closed != 0, check)) {
        return false;
    }
    madopilot_frame_t* after = nullptr;
    madopilot_error_t* error = nullptr;
    const madopilot_status_t status =
        api->session_acquire_frame(session, operation, &after, &error);
    const bool refused_closed = refused(status, &error, after, MADOPILOT_STATUS_CLOSED, check);
    const bool released = require_ok(api->frame_release(after), check);
    return refused_closed && released;
}

bool matching_flow(const char* package_path, bool mapping_only)
{
    madopilot_operation_t operation{};
    madopilot_operation_t refusing{};
    madopilot_frame_stamp_t stamp{};
    madopilot_status_t status;
    madopilot_cancellation_t* token = nullptr;
    madopilot_engine_t* engine = nullptr;
    madopilot_session_t* session = nullptr;
    madopilot_frame_t* frame = nullptr;
    madopilot_mapping_t* mapping = nullptr;
    madopilot_package_t* package = nullptr;
    madopilot_template_t* present = nullptr;
    madopilot_template_t* absent = nullptr;
    madopilot_result_t* found = nullptr;
    madopilot_result_t* missing = nullptr;
    madopilot_result_t* refused_result = nullptr;
    madopilot_error_t* error = nullptr;
    std::vector<std::uint8_t> scene(SCENE_BYTES);
    scene_fill_rgba(scene.data());
    operation.struct_size = static_cast<std::uint32_t>(sizeof(operation));

    {
        auto pixels = scene;
        madopilot_replay_frame_t supplied{};
        supplied.struct_size = static_cast<std::uint32_t>(sizeof(supplied));
        supplied.width = SCENE_WIDTH;
        supplied.height = SCENE_HEIGHT;
        supplied.stride = static_cast<std::uint64_t>(SCENE_WIDTH) * 4u;
        supplied.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
        supplied.continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
        supplied.pixels = {pixels.data(), pixels.size()};
        madopilot_source_t source{};
        source.struct_size = static_cast<std::uint32_t>(sizeof(source));
        source.kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
        source.frames = &supplied;
        source.frame_count = 1;
        source.frame_stride = sizeof(supplied);
        source.target_name = borrow("panel");
        status = api->engine_create(&source, &operation, &engine, nullptr);
        auto poison = static_cast<volatile std::uint8_t*>(pixels.data());
        std::fill(poison, poison + pixels.size(), std::uint8_t{0xa5});
        if (!require_ok(status, "matching-engine")) goto cleanup;
    }
    {
        madopilot_open_request_t open{};
        open.struct_size = static_cast<std::uint32_t>(sizeof(open));
        open.flags = MADOPILOT_OPEN_HAS_REQUIRED_FORMAT;
        open.required_format = MADOPILOT_PIXEL_FORMAT_RGBA8;
        if (!open_only_target(engine, &operation, &open, &session,
                              "matching-discover", "matching-open")) goto cleanup;
    }
    {
        madopilot_frame_info_t info{};
        madopilot_map_request_t request{};
        stamp.struct_size = static_cast<std::uint32_t>(sizeof(stamp));
        info.struct_size = static_cast<std::uint32_t>(sizeof(info));
        if (!require_ok(api->session_acquire_frame(session, &operation, &frame, nullptr),
                        "matching-frame") ||
            !require_ok(api->frame_stamp(frame, &stamp), "matching-frame") ||
            !require_ok(api->frame_describe(frame, &info), "matching-frame") ||
            !require(first_publication(stamp) && info.width == SCENE_WIDTH &&
                     info.height == SCENE_HEIGHT &&
                     same_rect(info.bounds, 0, 0, static_cast<std::int32_t>(SCENE_WIDTH),
                               static_cast<std::int32_t>(SCENE_HEIGHT)), "matching-frame")) {
            goto cleanup;
        }
        request.struct_size = static_cast<std::uint32_t>(sizeof(request));
        request.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
        if (!require_ok(api->frame_map(frame, &request, &operation, &mapping, nullptr),
                        "matching-map") ||
            !require(mapping_readable(mapping, stamp, scene), "matching-map")) goto cleanup;
    }
    if (mapping_only) {
        if (!closed_idempotently(session, &operation, "mapping-close")) goto cleanup;
        goto release_producers;
    }
    status = api->mapping_release(mapping);
    mapping = nullptr;
    if (!require_ok(status, "mapping-release")) goto cleanup;

    {
        madopilot_package_source_t source{};
        madopilot_package_info_t info{};
        source.struct_size = static_cast<std::uint32_t>(sizeof(source));
        source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
        source.path = borrow(package_path);
        info.struct_size = static_cast<std::uint32_t>(sizeof(info));
        if (!require_ok(api->package_load(engine, &source, &operation, &package, nullptr), "package") ||
            !require_ok(api->package_describe(package, &info), "package") ||
            !require(info.template_count == 2, "package") ||
            !require_ok(api->template_prepare_from_package(engine, package, borrow("panel.patch"),
                                                           &operation, &present, nullptr),
                        "template-present") ||
            !require_ok(api->template_prepare_from_package(engine, package, borrow("panel.absent"),
                                                           &operation, &absent, nullptr),
                        "template-absent")) goto cleanup;
    }
    {
        madopilot_find_request_t find{};
        find.struct_size = static_cast<std::uint32_t>(sizeof(find));
        find.frame = frame;
        find.tmpl = present;
        if (!require_ok(api->session_find(session, &find, &operation, &found, nullptr), "find-present") ||
            !require(planted_found(found, stamp), "find-present")) goto cleanup;
        find.tmpl = absent;
        if (!require_ok(api->session_find(session, &find, &operation, &missing, nullptr), "find-absent") ||
            !require(nothing_found(missing, stamp), "find-absent")) goto cleanup;
        std::puts("MADO_PROFILE_MATCHING=passed");

        find.tmpl = present;
        if (!cancelled_operation(refusing, &token, "cancellation")) goto cleanup;
        status = api->session_find(session, &find, &refusing, &refused_result, &error);
        if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED, "cancellation")) {
            goto cleanup;
        }
        std::puts("MADO_PROFILE_CANCELLATION=refused");
        if (!expired_operation(refusing, "deadline")) goto cleanup;
        status = api->session_find(session, &find, &refusing, &refused_result, &error);
        if (!refused(status, &error, refused_result, MADOPILOT_STATUS_DEADLINE_EXCEEDED,
                     "deadline")) goto cleanup;
        std::puts("MADO_PROFILE_DEADLINE=refused");
        refusing.cancellation = token;
        status = api->session_find(session, &find, &refusing, &refused_result, &error);
        if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED,
                     "cancellation-precedence")) goto cleanup;
    }
    if (!closed_idempotently(session, &operation, "close")) goto cleanup;
    std::puts("MADO_PROFILE_CLOSE=idempotent");

release_producers:
    status = api->template_release(present);
    present = nullptr;
    if (!require_ok(status, "retained")) goto cleanup;
    status = api->template_release(absent);
    absent = nullptr;
    if (!require_ok(status, "retained")) goto cleanup;
    status = api->package_release(package);
    package = nullptr;
    if (!require_ok(status, "retained")) goto cleanup;
    status = api->frame_release(frame);
    frame = nullptr;
    if (!require_ok(status, "retained")) goto cleanup;
    status = api->session_release(session);
    session = nullptr;
    if (!require_ok(status, "retained")) goto cleanup;
    status = api->engine_release(engine);
    engine = nullptr;
    if (!require_ok(status, "retained")) goto cleanup;
    if (!require(mapping_only ? mapping_readable(mapping, stamp, scene) :
                     planted_found(found, stamp) && nothing_found(missing, stamp), "retained")) {
        goto cleanup;
    }
    std::puts(mapping_only ? "MADO_PROFILE_MAPPING=retained" : "MADO_PROFILE_RETAINED=readable");

cleanup:
    require_ok(api->error_release(error), "release");
    require_ok(api->result_release(refused_result), "release");
    require_ok(api->result_release(missing), "release");
    require_ok(api->result_release(found), "release");
    require_ok(api->template_release(absent), "release");
    require_ok(api->template_release(present), "release");
    require_ok(api->package_release(package), "release");
    require_ok(api->mapping_release(mapping), "release");
    require_ok(api->frame_release(frame), "release");
    if (session != nullptr) require_ok(api->session_close(session, &operation, nullptr), "release");
    require_ok(api->session_release(session), "release");
    require_ok(api->engine_release(engine), "release");
    require_ok(api->cancellation_release(token), "release");
    return failed_check == nullptr;
}

bool ocr_flow(const madopilot_build_info_t& build, const char* model_root, const char* runtime_path)
{
    madopilot_operation_t operation{};
    madopilot_operation_t refusing{};
    madopilot_frame_stamp_t stamp{};
    madopilot_ocr_request_t request{};
    madopilot_status_t status;
    madopilot_cancellation_t* token = nullptr;
    madopilot_engine_t* engine = nullptr;
    madopilot_session_t* session = nullptr;
    madopilot_frame_t* frame = nullptr;
    madopilot_ocr_result_t* full = nullptr;
    madopilot_ocr_result_t* bounded = nullptr;
    madopilot_ocr_result_t* refused_result = nullptr;
    madopilot_error_t* error = nullptr;
    const std::string controlled_root = controlled_path(model_root);
    const std::string controlled_runtime = controlled_path(runtime_path);
    const auto whole = static_cast<std::int32_t>(blank_extent);
    operation.struct_size = static_cast<std::uint32_t>(sizeof(operation));
    if (!require(!controlled_root.empty() && !controlled_runtime.empty(), "ocr-prerequisite")) {
        goto cleanup;
    }

    {
        std::vector<std::uint8_t> blank(blank_bytes, 0);
        madopilot_replay_frame_t supplied{};
        supplied.struct_size = static_cast<std::uint32_t>(sizeof(supplied));
        supplied.width = blank_extent;
        supplied.height = blank_extent;
        supplied.stride = static_cast<std::uint64_t>(blank_extent) * 4u;
        supplied.format = MADOPILOT_PIXEL_FORMAT_BGRA8;
        supplied.continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
        supplied.pixels = {blank.data(), blank.size()};
        madopilot_source_t source{};
        source.struct_size = static_cast<std::uint32_t>(sizeof(source));
        source.kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
        source.frames = &supplied;
        source.frame_count = 1;
        source.frame_stride = sizeof(supplied);
        source.target_name = borrow("default-ocr-blank");
        madopilot_engine_options_t options{};
        options.struct_size = static_cast<std::uint32_t>(sizeof(options));
        madopilot_default_ocr_options_t default_ocr{};
        default_ocr.struct_size = static_cast<std::uint32_t>(sizeof(default_ocr));
        default_ocr.model_root = borrow(controlled_root.c_str());
        default_ocr.runtime_path = borrow(controlled_runtime.c_str());
        status = api->engine_create_with_default_ocr(&source, &options, &default_ocr,
                                                    &operation, &engine, nullptr);
        auto poison = static_cast<volatile std::uint8_t*>(blank.data());
        std::fill(poison, poison + blank.size(), std::uint8_t{0xa5});
        if (!require_ok(status, "ocr-engine")) goto cleanup;
    }
    {
        madopilot_ocr_provider_descriptor_t provider{};
        provider.struct_size = static_cast<std::uint32_t>(sizeof(provider));
        if (!require_ok(api->engine_ocr_provider_descriptor(engine, &provider), "ocr-provider") ||
            !require(provider.active_provider == MADOPILOT_OCR_EXECUTION_PROVIDER_CPU &&
                     provider.requested_policy == MADOPILOT_OCR_PROVIDER_POLICY_CPU &&
                     provider.initialization_fell_back == 0 &&
                     same_text(provider.runtime_profile, "onnxruntime-1.29.0-api17-cpu"),
                     "ocr-provider")) goto cleanup;
        madopilot_engine_capabilities_t capabilities{};
        capabilities.struct_size = static_cast<std::uint32_t>(sizeof(capabilities));
        if (!require_ok(api->engine_capabilities(engine, &capabilities), "ocr-engine") ||
            !require((capabilities.flags & MADOPILOT_ENGINE_HAS_OCR) != 0, "ocr-engine")) {
            goto cleanup;
        }
        madopilot_open_request_t open{};
        open.struct_size = static_cast<std::uint32_t>(sizeof(open));
        if (!open_only_target(engine, &operation, &open, &session, "ocr-discover", "ocr-open")) {
            goto cleanup;
        }
    }
    stamp.struct_size = static_cast<std::uint32_t>(sizeof(stamp));
    if (!require_ok(api->session_acquire_frame(session, &operation, &frame, nullptr), "ocr-frame") ||
        !require_ok(api->frame_stamp(frame, &stamp), "ocr-frame") ||
        !require(first_publication(stamp), "ocr-frame")) goto cleanup;

    request.struct_size = static_cast<std::uint32_t>(sizeof(request));
    request.frame = frame;
    request.model_id = build.default_ocr_model;
    request.backend_id = build.default_ocr_backend;
    request.backend_version = build.default_ocr_backend_version;
    request.output_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    if (!require_ok(api->session_recognize(session, &request, &operation, &full, nullptr), "ocr-full") ||
        !require(empty_recognition(full, stamp, 0, 0, whole, whole), "ocr-full")) goto cleanup;
    request.flags = MADOPILOT_OCR_HAS_REGION;
    request.region = {MADOPILOT_SPACE_CAPTURE_PIXELS, 8, 8, 40, 40};
    if (!require_ok(api->session_recognize(session, &request, &operation, &bounded, nullptr),
                    "ocr-region") ||
        !require(empty_recognition(bounded, stamp, 8, 8, 40, 40), "ocr-region")) goto cleanup;

    if (!cancelled_operation(refusing, &token, "ocr-cancellation")) goto cleanup;
    status = api->session_recognize(session, &request, &refusing, &refused_result, &error);
    if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED, "ocr-cancellation")) {
        goto cleanup;
    }
    if (!expired_operation(refusing, "ocr-deadline")) goto cleanup;
    status = api->session_recognize(session, &request, &refusing, &refused_result, &error);
    if (!refused(status, &error, refused_result, MADOPILOT_STATUS_DEADLINE_EXCEEDED,
                 "ocr-deadline")) goto cleanup;
    refusing.cancellation = token;
    status = api->session_recognize(session, &request, &refusing, &refused_result, &error);
    if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED,
                 "ocr-cancellation-precedence")) goto cleanup;
    if (!closed_idempotently(session, &operation, "ocr-close")) goto cleanup;

    status = api->frame_release(frame);
    frame = nullptr;
    if (!require_ok(status, "ocr-retained")) goto cleanup;
    status = api->session_release(session);
    session = nullptr;
    if (!require_ok(status, "ocr-retained")) goto cleanup;
    status = api->engine_release(engine);
    engine = nullptr;
    if (!require_ok(status, "ocr-retained")) goto cleanup;
    if (!require(empty_recognition(full, stamp, 0, 0, whole, whole) &&
                 empty_recognition(bounded, stamp, 8, 8, 40, 40), "ocr-retained")) goto cleanup;
    std::puts("MADO_PROFILE_OCR=passed");

cleanup:
    require_ok(api->error_release(error), "release");
    require_ok(api->ocr_result_release(refused_result), "release");
    require_ok(api->ocr_result_release(bounded), "release");
    require_ok(api->ocr_result_release(full), "release");
    require_ok(api->frame_release(frame), "release");
    if (session != nullptr) require_ok(api->session_close(session, &operation, nullptr), "release");
    require_ok(api->session_release(session), "release");
    require_ok(api->engine_release(engine), "release");
    require_ok(api->cancellation_release(token), "release");
    return failed_check == nullptr;
}

int finish()
{
    if (failed_check == nullptr) {
        std::puts("MADO_PROFILE_RESULT=passed");
        return 0;
    }
    std::printf("MADO_PROFILE_FAILURE=%s\n", failed_check);
    if (failed_status != MADOPILOT_STATUS_OK) {
        madopilot_str_t slug{};
        if (api != nullptr && api->status_text(failed_status, &slug) == MADOPILOT_STATUS_OK &&
            slug.data != nullptr) {
            std::printf("MADO_PROFILE_STATUS=%.*s\n", static_cast<int>(slug.len), slug.data);
        } else {
            std::printf("MADO_PROFILE_STATUS=%d\n", static_cast<int>(failed_status));
        }
    }
    std::puts("MADO_PROFILE_RESULT=failed");
    return 1;
}

} // namespace

int main(int argc, char** argv)
{
    const char* package = nullptr;
    const char* model_root = nullptr;
    const char* runtime = nullptr;
    int index = 1;
    std::setvbuf(stdout, nullptr, _IONBF, 0);
    for (; index + 1 < argc; index += 2) {
        const std::string_view flag(argv[index]);
        if (flag == "--package" && package == nullptr) {
            package = argv[index + 1];
        } else if (flag == "--model-root" && model_root == nullptr) {
            model_root = argv[index + 1];
        } else if (flag == "--runtime" && runtime == nullptr) {
            runtime = argv[index + 1];
        } else {
            break;
        }
    }
    if (index != argc || package == nullptr || model_root == nullptr || runtime == nullptr) {
        std::fputs("usage: --package <dir> --model-root <dir> --runtime <file>\n", stderr);
        fail("usage", MADOPILOT_STATUS_OK);
        return finish();
    }

    const madopilot_api_t* negotiated = nullptr;
    constexpr std::size_t known_extent = sizeof(madopilot_api_t);
    const madopilot_status_t status =
        madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, known_extent, &negotiated);
    if (!require_ok(status, "negotiate") || !require(negotiated != nullptr, "negotiate")) {
        return finish();
    }
    const std::size_t extent = std::min(known_extent, static_cast<std::size_t>(negotiated->struct_size));
    if (!require(negotiated->abi_major == MADOPILOT_ABI_MAJOR && negotiated->abi_minor >= 5u &&
                 extent >= MADOPILOT_API_SIZE_ENGINE_OCR_PROVIDER_DESCRIPTOR, "negotiate")) {
        return finish();
    }
    api = negotiated;
    madopilot_build_info_t build{};
    build.struct_size = static_cast<std::uint32_t>(sizeof(build));
    if (!require_ok(api->describe_build(&build), "build")) return finish();
    std::puts("MADO_PROFILE_CONSUMER=cpp-abi");
    std::printf("MADO_PROFILE_ABI=%u.%u\n", api->abi_major, api->abi_minor);
    std::printf("MADO_PROFILE_API_EXTENT=%zu\n", extent);
    if (matching_flow(package, false) && matching_flow(package, true)) {
        ocr_flow(build, model_root, runtime);
    }
    return finish();
}
