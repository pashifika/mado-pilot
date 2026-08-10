/*
 * Safe native common flow through the header-only C++ wrapper.
 *
 * `--check` is suitable for unattended CI: it creates the platform adapter and
 * reads only non-prompting permission/capability state. Passing one exact full
 * window title enables the end-to-end path: unique selection, capture, mapping,
 * one bounded input sequence under one activity tag, receipt inspection, a
 * newer-frame visual condition search, diagnostic drain, and explicit close.
 *
 * On Windows, ordinary targets are unknown-but-attemptable and report queue
 * admission; only the dedicated fixture reports protocol acknowledgement. A
 * receipt alone is never the visual success oracle.
 */

#include <chrono>
#include <cstdint>
#include <cstdio>
#include <string_view>
#include <thread>
#include <utility>
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <psapi.h>
#pragma comment(lib, "psapi.lib")
#elif defined(__APPLE__)
#include <sys/resource.h>
#endif


#include "madopilot/madopilot.hpp"
#include "../native-expected-condition.h"

namespace {

#if defined(_WIN32)
constexpr const char* EXAMPLE_NAME = "windows-native-input-cpp";
constexpr std::uint64_t REQUIRED_PAIRS =
    MADOPILOT_INPUT_PAIR_POINTER_WINDOW_MESSAGE |
    MADOPILOT_INPUT_PAIR_KEYBOARD_WINDOW_MESSAGE |
    MADOPILOT_INPUT_PAIR_TEXT_WINDOW_MESSAGE;
constexpr madopilot::InputDelivery DELIVERY =
    MADOPILOT_INPUT_DELIVERY_WINDOW_MESSAGE;
constexpr madopilot::InputAddressScope ADDRESS_SCOPE =
    MADOPILOT_INPUT_ADDRESS_EXACT_WINDOW;
constexpr bool ALLOWS_UNKNOWN = true;
constexpr bool READS_PERMISSIONS = false;
constexpr madopilot::FocusPolicy FOCUS = MADOPILOT_FOCUS_PRESERVE;
constexpr std::uint64_t EXPECTED_SUBMITTED = 16;

madopilot::Source native_source()
{
    return madopilot::Source::native_windows();
}
#elif defined(__APPLE__)
constexpr const char* EXAMPLE_NAME = "macos-native-input-cpp";
constexpr std::uint64_t REQUIRED_PAIRS =
    MADOPILOT_INPUT_PAIR_POINTER_SYSTEM |
    MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM;
constexpr madopilot::InputDelivery DELIVERY = MADOPILOT_INPUT_DELIVERY_SYSTEM;
constexpr madopilot::InputAddressScope ADDRESS_SCOPE =
    MADOPILOT_INPUT_ADDRESS_FOCUSED_SYSTEM;
constexpr bool ALLOWS_UNKNOWN = false;
constexpr madopilot::FocusPolicy FOCUS = MADOPILOT_FOCUS_ACTIVATE_IF_REQUIRED;
constexpr bool READS_PERMISSIONS = true;
constexpr std::uint64_t EXPECTED_SUBMITTED = 4;

madopilot::Source native_source()
{
    return madopilot::Source::native_macos();
}
#else
#error "native-input.cpp requires a MadoPilot release-target platform"
#endif

int failures = 0;
enum class RouteContract {
    platform_default,
    ordinary_window,
    acknowledged_fixture,
};

bool capability_matches_contract(const madopilot::InputCapability& capability,
                                 RouteContract contract)
{
    if (contract == RouteContract::ordinary_window) {
        return capability.support == MADOPILOT_CAPABILITY_UNKNOWN &&
               capability.evidence ==
                   MADOPILOT_SUBMISSION_EVIDENCE_TARGET_QUEUE_ADMISSION;
    }
    if (contract == RouteContract::acknowledged_fixture) {
        return capability.support == MADOPILOT_CAPABILITY_SUPPORTED &&
               capability.evidence ==
                   MADOPILOT_SUBMISSION_EVIDENCE_TARGET_PROTOCOL_ACKNOWLEDGEMENT;
    }
    return capability.support == MADOPILOT_CAPABILITY_SUPPORTED ||
           (ALLOWS_UNKNOWN &&
            capability.support == MADOPILOT_CAPABILITY_UNKNOWN);
}

bool descriptor_matches_contract(const madopilot::InputDescriptor& descriptor,
                                 RouteContract contract)
{
    if (contract == RouteContract::ordinary_window) {
        return (descriptor.unknown_pairs & REQUIRED_PAIRS) == REQUIRED_PAIRS &&
               (descriptor.known_pairs & REQUIRED_PAIRS) == 0 &&
               (descriptor.supported_pairs & REQUIRED_PAIRS) == 0;
    }
    if (contract == RouteContract::acknowledged_fixture) {
        return (descriptor.known_pairs & REQUIRED_PAIRS) == REQUIRED_PAIRS &&
               (descriptor.supported_pairs & REQUIRED_PAIRS) == REQUIRED_PAIRS &&
               (descriptor.unknown_pairs & REQUIRED_PAIRS) == 0;
    }
    const auto attemptable_pairs =
        descriptor.supported_pairs |
        (ALLOWS_UNKNOWN ? descriptor.unknown_pairs : UINT64_C(0));
    return (attemptable_pairs & REQUIRED_PAIRS) == REQUIRED_PAIRS;
}

const char* route_contract_name(RouteContract contract)
{
    switch (contract) {
    case RouteContract::ordinary_window:
        return "ordinary";
    case RouteContract::acknowledged_fixture:
        return "acknowledged";
    case RouteContract::platform_default:
        return "platform-default";
    }
    return "invalid";
}

constexpr std::uint64_t DIAGNOSTIC_ACTIVITY_TAG = UINT64_C(0x4d50494e505554a1);
constexpr std::uint32_t DIAGNOSTIC_CAPACITY = UINT32_C(256);
std::uint64_t peak_resident_bytes()
{
#if defined(_WIN32)
    PROCESS_MEMORY_COUNTERS counters{};
    counters.cb = sizeof(counters);
    if (!GetProcessMemoryInfo(GetCurrentProcess(), &counters, sizeof(counters))) {
        return 0;
    }
    return static_cast<std::uint64_t>(counters.PeakWorkingSetSize);
#elif defined(__APPLE__)
    rusage usage{};
    if (getrusage(RUSAGE_SELF, &usage) != 0) {
        return 0;
    }
    return static_cast<std::uint64_t>(usage.ru_maxrss);
#else
    return 0;
#endif
}

void report_peak_resident_bytes()
{
    std::printf("peak resident: %llu byte(s)\n",
                static_cast<unsigned long long>(peak_resident_bytes()));
}


bool expect(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAILED: %s\n", message);
        ++failures;
    }
    return condition;
}

bool report_failure(const char* operation, const madopilot::Error& error)
{
    std::fprintf(stderr, "%s failed with status %d", operation,
                 static_cast<int>(error.status()));
    if (!error.message().empty()) {
        std::fprintf(stderr, ": %s", error.message().c_str());
    }
    std::fputc('\n', stderr);
    ++failures;
    return false;
}

bool bounded_operation(const madopilot::Api& api, std::uint64_t budget_nanos,
                       madopilot::Operation& operation)
{
    const auto now = api.clock_now();
    if (!now) {
        return report_failure("clock_now", now.error());
    }
    operation = madopilot::Operation{};
    operation.deadline(now.value() + budget_nanos);
    operation.activity_tag(DIAGNOSTIC_ACTIVITY_TAG);
    return true;
}

bool drain_diagnostics(madopilot::DiagnosticReader& reader, bool require_mapping)
{
    std::uint64_t records = 0;
    std::uint64_t normal = 0;
    std::uint64_t debug = 0;
    std::uint64_t discarded_normal = 0;
    std::uint64_t discarded_debug = 0;
    std::uint64_t mappings = 0;

    for (;;) {
        auto drained = reader.drain();
        if (!drained) {
            return report_failure("DiagnosticReader::drain", drained.error());
        }
        auto drain = drained.take();
        if (drain.state == MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM) {
            if (!expect(!drain.batch.has_value(),
                        "end-of-stream has no diagnostic batch")) {
                return false;
            }
            break;
        }
        if (!expect(drain.state == MADOPILOT_DIAGNOSTIC_DRAIN_BATCH &&
                        drain.batch.has_value(),
                    "a sealed diagnostic reader yields batches then ends")) {
            return false;
        }

        madopilot::DiagnosticBatch batch = std::move(*drain.batch);
        const auto described = batch.describe();
        if (!described) {
            return report_failure("DiagnosticBatch::describe",
                                  described.error());
        }
        discarded_normal += described.value().discarded_normal;
        discarded_debug += described.value().discarded_debug;
        for (std::uint64_t index = 0;
             index < described.value().record_count; ++index) {
            const auto record = batch.record_at(static_cast<std::size_t>(index));
            if (!record) {
                return report_failure("DiagnosticBatch::record_at",
                                      record.error());
            }
            if (!expect((record.value().flags &
                         MADOPILOT_DIAGNOSTIC_RECORD_HAS_ACTIVITY) != 0 &&
                            record.value().activity_tag ==
                                DIAGNOSTIC_ACTIVITY_TAG,
                        "every diagnostic record retains the caller activity")) {
                return false;
            }
            ++records;
            normal += record.value().level ==
                      MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL;
            debug += record.value().level == MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG;
            if (record.value().kind == MADOPILOT_DIAGNOSTIC_KIND_MAPPING) {
                constexpr std::uint32_t required =
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET |
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME |
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE |
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE;
                if (!expect(
                        (record.value().flags & required) == required &&
                            record.value().source_space ==
                                MADOPILOT_SPACE_CAPTURE_PIXELS &&
                            record.value().destination_space ==
                                MADOPILOT_SPACE_CAPTURE_PIXELS,
                        "mapping diagnostics expose copied identity and spaces")) {
                    return false;
                }
                ++mappings;
            }
        }
    }

    std::printf("diagnostics: %llu record(s), normal %llu, debug %llu, "
                "discarded-normal %llu, discarded-debug %llu\n",
                static_cast<unsigned long long>(records),
                static_cast<unsigned long long>(normal),
                static_cast<unsigned long long>(debug),
                static_cast<unsigned long long>(discarded_normal),
                static_cast<unsigned long long>(discarded_debug));
    return expect(records != 0,
                  "the enabled diagnostic stream retained records") &&
           (!require_mapping ||
            expect(mappings != 0,
                   "the mapped frame emitted a debug mapping fact"));
}

bool same_frame_stamp(const madopilot::FrameStamp& left,
                      const madopilot::FrameStamp& right)
{
    return left.stream == right.stream && left.epoch == right.epoch &&
           left.sequence == right.sequence && left.geometry == right.geometry;
}

bool strictly_newer_frame(const madopilot::FrameStamp& candidate,
                          const madopilot::FrameStamp& before)
{
    return candidate.stream == before.stream &&
           (candidate.epoch > before.epoch ||
            (candidate.epoch == before.epoch &&
             candidate.sequence > before.sequence));
}

bool observe_expected_condition(const madopilot::Api& api,
                                madopilot::Session& session,
                                const madopilot::FrameStamp& before)
{
    madopilot::Operation operation;
    if (!bounded_operation(api, UINT64_C(5000000000), operation)) {
        return false;
    }

    for (;;) {
        auto acquired = session.acquire_frame(operation);
        if (!acquired) {
            return report_failure("Session::acquire_frame after input",
                                  acquired.error());
        }
        madopilot::Frame frame = acquired.take();
        const auto frame_stamp = frame.stamp();
        if (!frame_stamp) {
            return report_failure("Frame::stamp after input", frame_stamp.error());
        }
        if (!expect(frame_stamp.value().stream == before.stream,
                    "the observation remains correlated to the source stream")) {
            return false;
        }
        if (!strictly_newer_frame(frame_stamp.value(), before)) {
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
            continue;
        }

        madopilot::MapRequest request;
        request.format(MADOPILOT_PIXEL_FORMAT_BGRA8);
        auto mapped = frame.map(request, operation);
        if (!mapped) {
            return report_failure("Frame::map after input", mapped.error());
        }
        madopilot::Mapping mapping = mapped.take();
        const auto mapping_stamp = mapping.stamp();
        if (!mapping_stamp) {
            return report_failure("Mapping::stamp after input",
                                  mapping_stamp.error());
        }
        if (!expect(same_frame_stamp(mapping_stamp.value(), frame_stamp.value()),
                    "the visual search remains correlated to the observed frame")) {
            return false;
        }
        const auto image = mapping.describe();
        if (!image) {
            return report_failure("Mapping::describe after input", image.error());
        }
        const bool matched =
            image.value().format == MADOPILOT_PIXEL_FORMAT_BGRA8 &&
            madopilot_example_expected_condition_matches(
                image.value().bytes.data(), image.value().bytes.size(),
                image.value().stride, image.value().width, image.value().height);
        if (matched) {
            std::printf("expected condition: stream %llu epoch %llu sequence %llu\n",
                        static_cast<unsigned long long>(frame_stamp.value().stream),
                        static_cast<unsigned long long>(frame_stamp.value().epoch),
                        static_cast<unsigned long long>(frame_stamp.value().sequence));
            return true;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
}

bool probe_permissions(madopilot::Engine& engine,
                       const madopilot::Operation& operation,
                       bool require_granted)
{
    constexpr madopilot::PermissionKind KINDS[] = {
        MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE,
        MADOPILOT_PERMISSION_KIND_INPUT_CONTROL,
    };

    for (const auto kind : KINDS) {
        const auto permission = engine.permission(kind, operation);
        if (READS_PERMISSIONS) {
            if (!permission) {
                return report_failure("engine.permission", permission.error());
            }
            std::printf("permission kind %d state %d\n",
                        static_cast<int>(permission.value().kind),
                        static_cast<int>(permission.value().state));
            if (require_granted &&
                !expect(permission.value().state ==
                            MADOPILOT_PERMISSION_STATE_GRANTED,
                        "the fixture flow requires granted native permissions")) {
                return false;
            }
        } else {
            if (!expect(!permission,
                        "an adapter without permission probes refuses the call")) {
                return false;
            }
            if (!expect(permission.status() == MADOPILOT_STATUS_UNSUPPORTED,
                        "an unavailable permission probe reports unsupported")) {
                return false;
            }
            std::printf("permission kind %d unavailable (status %d)\n",
                        static_cast<int>(kind),
                        static_cast<int>(permission.status()));
        }
    }

    return true;
}

bool select_target(madopilot::TargetList& targets, std::string_view title,
                   RouteContract contract, std::size_t& selected,
                   std::uint64_t& target_id,
                   madopilot::SubmissionEvidence& evidence)
{
    const auto count = targets.count();
    if (!count) {
        return report_failure("TargetList::count", count.error());
    }
    std::printf("discovered: %zu target(s)\n", count.value());

    std::size_t matches = 0;
    for (std::size_t index = 0; index < count.value(); ++index) {
        const auto target = targets.at(index);
        if (!target) {
            return report_failure("TargetList::at", target.error());
        }
        if (target.value().kind == MADOPILOT_TARGET_KIND_WINDOW &&
            target.value().name.view() == title) {
            selected = index;
            target_id = target.value().target;
            ++matches;
        }
    }

    if (!expect(matches == 1,
                "the full title must identify exactly one window before input exists")) {
        return false;
    }

    const auto target = targets.at(selected);
    if (!target) {
        return report_failure("TargetList::at", target.error());
    }
    if (!expect(target.value().kind == MADOPILOT_TARGET_KIND_WINDOW,
                "the selected target remains a window") ||
        !expect(target.value().capture == MADOPILOT_CAPABILITY_SUPPORTED,
                "the selected target supports capture")) {
        return false;
    }

    constexpr madopilot::InputOperationKind OPERATIONS[] = {
        MADOPILOT_INPUT_OPERATION_POINTER,
        MADOPILOT_INPUT_OPERATION_KEYBOARD,
#if defined(_WIN32)
        MADOPILOT_INPUT_OPERATION_TEXT,
#endif
    };
    for (const auto operation : OPERATIONS) {
        const auto capability =
            targets.input_capability(selected, operation, DELIVERY);
        if (!capability) {
            return report_failure("TargetList::input_capability",
                                  capability.error());
        }
        if (!expect(capability.value().target == target_id &&
                        capability_matches_contract(capability.value(), contract),
                    "the selected target exposes the expected input contract") ||
            !expect(capability.value().address_scope == ADDRESS_SCOPE &&
                        capability.value().evidence.has_value(),
                    "the route reports its scope and strongest evidence")) {
            return false;
        }
        if (evidence == MADOPILOT_SUBMISSION_EVIDENCE_NONE) {
            evidence = *capability.value().evidence;
        } else if (!expect(evidence == *capability.value().evidence,
                           "required pairs share one route evidence level")) {
            return false;
        }
        if (operation == MADOPILOT_INPUT_OPERATION_POINTER &&
            !expect((capability.value().pointer_spaces &
                     (std::uint32_t{1} <<
                      MADOPILOT_SPACE_CAPTURE_PIXELS)) != 0,
                    "the route accepts capture-pixel pointer coordinates")) {
            return false;
        }
    }

    std::printf("selected: index %zu target %llu\n", selected,
                static_cast<unsigned long long>(target_id));
    return true;
}

#if defined(_WIN32)
bool request_projection_matches(const madopilot::InputRequest& request,
                                double centre_x, double centre_y)
{
    const auto projected = request.to_c();
    const auto& value = projected.value();
    if (!expect(value.event_count == EXPECTED_SUBMITTED &&
                    value.events != nullptr &&
                    value.event_stride == sizeof(madopilot_input_event_t),
                "the C++ request projects all Windows event records")) {
        return false;
    }
    const auto& events = value.events;
    if (!expect(events[0].kind == MADOPILOT_INPUT_EVENT_POINTER_MOVE &&
                    events[0].space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
                    events[0].x == centre_x && events[0].y == centre_y &&
                    events[1].kind == MADOPILOT_INPUT_EVENT_POINTER_PRESS &&
                    events[1].button == MADOPILOT_POINTER_BUTTON_PRIMARY &&
                    events[2].kind == MADOPILOT_INPUT_EVENT_POINTER_RELEASE &&
                    events[2].button == MADOPILOT_POINTER_BUTTON_PRIMARY &&
                    events[3].kind == MADOPILOT_INPUT_EVENT_POINTER_PRESS &&
                    events[3].button == MADOPILOT_POINTER_BUTTON_SECONDARY &&
                    events[4].kind == MADOPILOT_INPUT_EVENT_POINTER_RELEASE &&
                    events[4].button == MADOPILOT_POINTER_BUTTON_SECONDARY &&
                    events[5].kind == MADOPILOT_INPUT_EVENT_POINTER_PRESS &&
                    events[5].button == MADOPILOT_POINTER_BUTTON_MIDDLE &&
                    events[6].kind == MADOPILOT_INPUT_EVENT_POINTER_RELEASE &&
                    events[6].button == MADOPILOT_POINTER_BUTTON_MIDDLE &&
                    events[7].kind == MADOPILOT_INPUT_EVENT_POINTER_SCROLL &&
                    events[7].horizontal == -1 && events[7].vertical == 2,
                "the C++ request preserves every pointer and wheel variant")) {
        return false;
    }
    if (!expect(events[8].kind == MADOPILOT_INPUT_EVENT_KEY_PRESS &&
                    events[8].key == MADOPILOT_KEY_FUNCTION &&
                    events[8].key_value == 6 &&
                    events[9].kind == MADOPILOT_INPUT_EVENT_KEY_RELEASE &&
                    events[9].key == MADOPILOT_KEY_FUNCTION &&
                    events[9].key_value == 6 &&
                    events[10].kind == MADOPILOT_INPUT_EVENT_KEY_PRESS &&
                    events[10].key == MADOPILOT_KEY_MODIFIER &&
                    events[10].key_value == MADOPILOT_MODIFIER_CONTROL &&
                    events[11].kind == MADOPILOT_INPUT_EVENT_KEY_PRESS &&
                    events[11].key == MADOPILOT_KEY_CHARACTER &&
                    events[11].key_value == static_cast<std::uint32_t>('m') &&
                    events[12].kind == MADOPILOT_INPUT_EVENT_KEY_RELEASE &&
                    events[12].key == MADOPILOT_KEY_CHARACTER &&
                    events[12].key_value == static_cast<std::uint32_t>('m') &&
                    events[13].kind == MADOPILOT_INPUT_EVENT_KEY_RELEASE &&
                    events[13].key == MADOPILOT_KEY_MODIFIER &&
                    events[13].key_value == MADOPILOT_MODIFIER_CONTROL,
                "the C++ request preserves the function key and ordered chord")) {
        return false;
    }
    const std::string_view expected_text{"A\xF0\x9F\x98\x80"};
    return expect(events[14].kind == MADOPILOT_INPUT_EVENT_TEXT &&
                      events[14].text.data != nullptr &&
                      std::string_view(events[14].text.data,
                                       events[14].text.len) == expected_text &&
                      events[15].kind == MADOPILOT_INPUT_EVENT_DELAY &&
                      events[15].delay_nanos == UINT64_C(20000000),
                  "the C++ request preserves Unicode text and bounded delay");
}
#endif

bool exercise_session(const madopilot::Api& api, madopilot::Session& session,
                      madopilot::SubmissionEvidence evidence,
                      RouteContract contract)
{
    const auto session_info = session.describe();
    if (!session_info) {
        return report_failure("Session::describe", session_info.error());
    }
    if (!expect(session_info.value().accepts_input == 1,
                "the opened session established input")) {
        return false;
    }
    std::printf("session: stream %llu target %llu input %d\n",
                static_cast<unsigned long long>(session_info.value().stream),
                static_cast<unsigned long long>(session_info.value().target),
                static_cast<int>(session_info.value().accepts_input));

    const auto descriptor = session.input_descriptor();
    if (!descriptor) {
        return report_failure("Session::input_descriptor", descriptor.error());
    }
    if (!expect(descriptor_matches_contract(descriptor.value(), contract),
                "the session retains the expected required input pairs")) {
        return false;
    }

    madopilot::Operation operation;
    if (!bounded_operation(api, UINT64_C(5000000000), operation)) {
        return false;
    }
    auto acquired = session.acquire_frame(operation);
    if (!acquired) {
        return report_failure("Session::acquire_frame", acquired.error());
    }
    madopilot::Frame frame = acquired.take();

    const auto frame_info = frame.describe();
    if (!frame_info) {
        return report_failure("Frame::describe", frame_info.error());
    }
    const auto frame_stamp = frame.stamp();
    if (!frame_stamp) {
        return report_failure("Frame::stamp", frame_stamp.error());
    }
    std::printf("frame: epoch %llu sequence %llu geometry %llu %ux%u\n",
                static_cast<unsigned long long>(frame_stamp.value().epoch),
                static_cast<unsigned long long>(frame_stamp.value().sequence),
                static_cast<unsigned long long>(frame_stamp.value().geometry),
                static_cast<unsigned>(frame_info.value().width),
                static_cast<unsigned>(frame_info.value().height));

    madopilot::MapRequest map_request;
    map_request.format(MADOPILOT_PIXEL_FORMAT_BGRA8);
    if (!bounded_operation(api, UINT64_C(5000000000), operation)) {
        return false;
    }
    auto mapped = frame.map(map_request, operation);
    if (!mapped) {
        return report_failure("Frame::map", mapped.error());
    }
    madopilot::Mapping mapping = mapped.take();
    const auto image = mapping.describe();
    if (!image) {
        return report_failure("Mapping::describe", image.error());
    }
    if (!expect(!image.value().bytes.empty(),
                "the native frame maps to readable bytes") ||
        !expect(image.value().format == MADOPILOT_PIXEL_FORMAT_BGRA8 &&
                    !madopilot_example_expected_condition_matches(
                        image.value().bytes.data(), image.value().bytes.size(),
                        image.value().stride, image.value().width,
                        image.value().height),
                "the expected visual condition is absent before input")) {
        return false;
    }
    std::printf("mapping: %zu byte(s)\n", image.value().bytes.size());

    const double centre_x = static_cast<double>(frame_info.value().width) / 2.0;
    const double centre_y = static_cast<double>(frame_info.value().height) / 2.0;
    madopilot::InputRequest request;
#if defined(_WIN32)
    request.event(madopilot::InputEvent::pointer_move(
                      MADOPILOT_SPACE_CAPTURE_PIXELS, centre_x, centre_y))
        .event(madopilot::InputEvent::pointer_press(
            MADOPILOT_POINTER_BUTTON_PRIMARY))
        .event(madopilot::InputEvent::pointer_release(
            MADOPILOT_POINTER_BUTTON_PRIMARY))
        .event(madopilot::InputEvent::pointer_press(
            MADOPILOT_POINTER_BUTTON_SECONDARY))
        .event(madopilot::InputEvent::pointer_release(
            MADOPILOT_POINTER_BUTTON_SECONDARY))
        .event(madopilot::InputEvent::pointer_press(
            MADOPILOT_POINTER_BUTTON_MIDDLE))
        .event(madopilot::InputEvent::pointer_release(
            MADOPILOT_POINTER_BUTTON_MIDDLE))
        .event(madopilot::InputEvent::pointer_scroll(-1, 2))
        .event(madopilot::InputEvent::key_press(MADOPILOT_KEY_FUNCTION, 6))
        .event(madopilot::InputEvent::key_release(MADOPILOT_KEY_FUNCTION, 6))
        .event(madopilot::InputEvent::key_press(
            MADOPILOT_KEY_MODIFIER, MADOPILOT_MODIFIER_CONTROL))
        .event(madopilot::InputEvent::key_press(
            MADOPILOT_KEY_CHARACTER, static_cast<std::uint32_t>('m')))
        .event(madopilot::InputEvent::key_release(
            MADOPILOT_KEY_CHARACTER, static_cast<std::uint32_t>('m')))
        .event(madopilot::InputEvent::key_release(
            MADOPILOT_KEY_MODIFIER, MADOPILOT_MODIFIER_CONTROL))
        .event(madopilot::InputEvent::text("A\xF0\x9F\x98\x80"))
        .event(madopilot::InputEvent::delay(UINT64_C(20000000)));
#else
    request.event(madopilot::InputEvent::pointer_move(
                      MADOPILOT_SPACE_CAPTURE_PIXELS, centre_x, centre_y))
        .event(madopilot::InputEvent::key_press(MADOPILOT_KEY_CHARACTER,
                                                static_cast<std::uint32_t>('m')))
        .event(madopilot::InputEvent::key_release(MADOPILOT_KEY_CHARACTER,
                                                  static_cast<std::uint32_t>('m')))
        .event(madopilot::InputEvent::delay(UINT64_C(20000000)));
#endif
    request.delivery(DELIVERY)
        .focus_policy(FOCUS)
        .geometry_policy(MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED)
        .source_frame(frame);
#if defined(_WIN32)
    if (!request_projection_matches(request, centre_x, centre_y)) {
        return false;
    }
#endif

    if (!bounded_operation(api, UINT64_C(2000000000), operation)) {
        return false;
    }
    const auto send_started = std::chrono::steady_clock::now();
    auto sent = session.send_input(request, operation);
    if (!sent) {
        return report_failure("Session::send_input", sent.error());
    }
    if (!expect(std::chrono::steady_clock::now() - send_started >=
                    std::chrono::milliseconds(20),
                "the native C++ flow observes its bounded delay event")) {
        return false;
    }
    madopilot::InputReceipt receipt = sent.take();
    const auto info = receipt.describe();
    if (!info) {
        return report_failure("InputReceipt::describe", info.error());
    }
    std::printf("receipt: outcome %d submitted %llu evidence %d cleanup %d\n",
                static_cast<int>(info.value().outcome),
                static_cast<unsigned long long>(info.value().submitted),
                info.value().evidence
                    ? static_cast<int>(*info.value().evidence)
                    : 0,
                static_cast<int>(info.value().cleanup));
    if (!expect(info.value().outcome == MADOPILOT_SEQUENCE_COMPLETE &&
                    info.value().submitted == EXPECTED_SUBMITTED &&
                    info.value().selected_route.has_value() &&
                    *info.value().selected_route == DELIVERY &&
                    info.value().address_scope == ADDRESS_SCOPE &&
                    info.value().evidence.has_value() &&
                    *info.value().evidence == evidence,
                "the bounded sequence was submitted exactly once with "
                "truthful route evidence")) {
        return false;
    }

    const madopilot::FrameStamp before = frame_stamp.value();
    mapping.reset();
    frame.reset();
    return observe_expected_condition(api, session, before);
}

bool run_native(const madopilot::Api& api, madopilot::Engine& engine,
                std::string_view title, bool check_only, RouteContract contract)
{
    const auto capabilities = engine.capabilities();
    if (!capabilities) {
        return report_failure("Engine::capabilities", capabilities.error());
    }
    if (!expect(capabilities.value().delivers_input(),
                "the native engine exposes input capability") ||
        !expect(capabilities.value().reads_permissions() == READS_PERMISSIONS,
                "the native engine reports its permission behavior")) {
        return false;
    }
    std::printf("engine capabilities: 0x%x\n",
                static_cast<unsigned>(capabilities.value().flags));

    madopilot::Operation operation;
    if (!bounded_operation(api, UINT64_C(5000000000), operation) ||
        !probe_permissions(engine, operation, !check_only)) {
        return false;
    }
    if (check_only) {
        return true;
    }

    if (!bounded_operation(api, UINT64_C(5000000000), operation)) {
        return false;
    }
    auto discovered = engine.discover(operation);
    if (!discovered) {
        return report_failure("Engine::discover", discovered.error());
    }
    madopilot::TargetList targets = discovered.take();
    std::size_t selected = 0;
    std::uint64_t target_id = 0;
    madopilot::SubmissionEvidence evidence =
        MADOPILOT_SUBMISSION_EVIDENCE_NONE;
    if (!select_target(targets, title, contract, selected, target_id, evidence)) {
        return false;
    }

    const auto live_descriptor = engine.input_descriptor(targets, selected, operation);
    if (!live_descriptor) {
        return report_failure("Engine::input_descriptor", live_descriptor.error());
    }
    if (!expect(descriptor_matches_contract(live_descriptor.value(), contract),
                "the live descriptor retains the expected required input pairs")) {
        return false;
    }

    madopilot::InputOpenRequest input_open;
    input_open.requirement(MADOPILOT_INPUT_REQUIRED).require_pairs(REQUIRED_PAIRS);
    madopilot::OpenRequest open;
    open.input(input_open);
    if (!bounded_operation(api, UINT64_C(5000000000), operation)) {
        return false;
    }
    auto opened = engine.open_session(targets, selected, open, operation);
    if (!opened) {
        return report_failure("Engine::open_session", opened.error());
    }
    targets.reset();
    madopilot::Session session = opened.take();

    const bool exercised = exercise_session(api, session, evidence, contract);
    bool closed = false;
    if (bounded_operation(api, UINT64_C(2000000000), operation)) {
        const auto close = session.close(operation);
        if (close) {
            closed = true;
        } else {
            report_failure("Session::close", close.error());
        }
    }
    return exercised && closed;
}

} // namespace

int main(int argc, char** argv)
{
    bool check_only = false;
    bool load_only = false;
    std::string_view title;
    RouteContract contract = RouteContract::platform_default;
    if (argc == 2 && std::string_view(argv[1]) == "--load-check") {
        load_only = true;
    } else if (argc == 2 && std::string_view(argv[1]) == "--check") {
        check_only = true;
    } else if (argc == 3 && std::string_view(argv[1]) == "--ordinary" &&
               argv[2][0] != '\0') {
        contract = RouteContract::ordinary_window;
        title = argv[2];
    } else if (argc == 3 &&
               std::string_view(argv[1]) == "--acknowledged" &&
               argv[2][0] != '\0') {
        contract = RouteContract::acknowledged_fixture;
        title = argv[2];
    } else if (argc == 2 && argv[1][0] != '\0') {
        title = argv[1];
    } else {
        std::fprintf(stderr,
                     "usage: %s --load-check | --check | [--ordinary | "
                     "--acknowledged] \"<full fixture window title>\"\n",
                     argv[0]);
        return 2;
    }

    auto loaded = madopilot::Api::load();
    if (!loaded) {
        report_failure("madopilot::Api::load", loaded.error());
        return 1;
    }
    const madopilot::Api api = loaded.take();
    if (!expect(api.extent() >= MADOPILOT_API_SIZE_DIAGNOSTIC_BATCH_RECORD_AT,
                "the negotiated table contains the complete ABI 1.2 suffix")) {
        return 1;
    }

    const auto build = api.describe_build();
    if (!build) {
        report_failure("Api::describe_build", build.error());
        return 1;
    }
    std::printf("%s: abi %u.%u table %u\n", EXAMPLE_NAME,
                static_cast<unsigned>(build.value().abi_major),
                static_cast<unsigned>(build.value().abi_minor),
                static_cast<unsigned>(build.value().table_size));
    if (contract != RouteContract::platform_default) {
        std::printf("contract: %s\n", route_contract_name(contract));
    }
    if (load_only) {
        report_peak_resident_bytes();
        std::printf("%s complete (load check)\n", EXAMPLE_NAME);
        return 0;
    }

    madopilot::Operation operation;
    if (!bounded_operation(api, UINT64_C(5000000000), operation)) {
        return 1;
    }
    const madopilot::Source source = native_source();
    madopilot::EngineOptions engine_options;
    engine_options.diagnostics(MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG,
                               DIAGNOSTIC_CAPACITY);
    auto created = api.create_engine(source, engine_options, operation);
    if (!created) {
        report_failure("Api::create_engine", created.error());
        return 1;
    }
    madopilot::Engine engine = created.take();
    auto taken = engine.take_diagnostic_reader();
    if (!taken) {
        report_failure("Engine::take_diagnostic_reader", taken.error());
        return 1;
    }
    auto optional_reader = taken.take();
    if (!expect(optional_reader.has_value(),
                "an enabled engine exposes one diagnostic reader")) {
        return 1;
    }
    madopilot::DiagnosticReader diagnostics = std::move(*optional_reader);

    const bool worked = run_native(api, engine, title, check_only, contract);
    engine.reset();
    const bool diagnostics_drained = drain_diagnostics(diagnostics, !check_only);
    if (!worked || !diagnostics_drained || failures != 0) {
        return 1;
    }
    report_peak_resident_bytes();

    std::printf("%s complete%s\n", EXAMPLE_NAME,
                check_only ? " (non-prompting check)" : "");
    return 0;
}
