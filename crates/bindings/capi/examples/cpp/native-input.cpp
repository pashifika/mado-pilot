/*
 * Safe native common flow through the header-only C++ wrapper.
 *
 * `--check` is suitable for unattended CI: it creates the platform adapter and
 * reads only non-prompting permission/capability state. Passing one exact full
 * fixture-window title enables the end-to-end path: unique selection, capture,
 * mapping, one bounded input sequence, receipt inspection, and explicit close.
 */

#include <cstdint>
#include <cstdio>
#include <string_view>
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <psapi.h>
#pragma comment(lib, "psapi.lib")
#elif defined(__APPLE__)
#include <sys/resource.h>
#endif


#include "madopilot/madopilot.hpp"

namespace {

#if defined(_WIN32)
constexpr const char* EXAMPLE_NAME = "windows-native-input-cpp";
constexpr std::uint64_t REQUIRED_PAIRS =
    MADOPILOT_INPUT_PAIR_POINTER_BACKGROUND |
    MADOPILOT_INPUT_PAIR_KEYBOARD_BACKGROUND;
constexpr madopilot::InputDelivery DELIVERY =
    MADOPILOT_INPUT_DELIVERY_BACKGROUND_TARGET;
constexpr madopilot::FocusPolicy FOCUS = MADOPILOT_FOCUS_PRESERVE;
constexpr bool READS_PERMISSIONS = false;

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
constexpr madopilot::FocusPolicy FOCUS = MADOPILOT_FOCUS_ACTIVATE_IF_REQUIRED;
constexpr bool READS_PERMISSIONS = true;

madopilot::Source native_source()
{
    return madopilot::Source::native_macos();
}
#else
#error "native-input.cpp requires a MadoPilot release-target platform"
#endif

int failures = 0;
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
    return true;
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
                   std::size_t& selected,
                   madopilot::TargetCapability& capability)
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
            ++matches;
        }
    }

    if (!expect(matches == 1,
                "the full title must identify exactly one window before input exists")) {
        return false;
    }

    const auto selected_capability = targets.capability(selected);
    if (!selected_capability) {
        return report_failure("TargetList::capability",
                              selected_capability.error());
    }
    capability = selected_capability.value();

    if (!expect(capability.kind.has_value() &&
                    *capability.kind == MADOPILOT_TARGET_KIND_WINDOW,
                "the selected target remains a window") ||
        !expect(capability.capture == MADOPILOT_CAPABILITY_SUPPORTED,
                "the selected target supports capture") ||
        !expect((capability.input_pairs & REQUIRED_PAIRS) == REQUIRED_PAIRS,
                "the selected target exposes every required input pair") ||
        !expect((capability.pointer_spaces &
                 (std::uint32_t{1} << MADOPILOT_SPACE_CAPTURE_PIXELS)) != 0,
                "the selected target accepts capture-pixel pointer coordinates")) {
        return false;
    }

    std::printf("selected: index %zu target %llu\n", selected,
                static_cast<unsigned long long>(capability.target));
    return true;
}

bool exercise_session(const madopilot::Api& api, madopilot::Session& session)
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
    if (!expect((descriptor.value().pairs & REQUIRED_PAIRS) == REQUIRED_PAIRS,
                "the session retains every required input pair")) {
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
    map_request.format(frame_info.value().format);
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
                "the native frame maps to readable bytes")) {
        return false;
    }
    std::printf("mapping: %zu byte(s)\n", image.value().bytes.size());

    const double centre_x = static_cast<double>(frame_info.value().width) / 2.0;
    const double centre_y = static_cast<double>(frame_info.value().height) / 2.0;
    madopilot::InputRequest request;
    request.event(madopilot::InputEvent::pointer_move(
                      MADOPILOT_SPACE_CAPTURE_PIXELS, centre_x, centre_y))
        .event(madopilot::InputEvent::key_press(MADOPILOT_KEY_CHARACTER,
                                                static_cast<std::uint32_t>('m')))
        .event(madopilot::InputEvent::key_release(MADOPILOT_KEY_CHARACTER,
                                                  static_cast<std::uint32_t>('m')))
        .event(madopilot::InputEvent::delay(UINT64_C(20000000)))
        .event(madopilot::InputEvent::key_press(MADOPILOT_KEY_CHARACTER,
                                                static_cast<std::uint32_t>('p')))
        .event(madopilot::InputEvent::key_release(MADOPILOT_KEY_CHARACTER,
                                                  static_cast<std::uint32_t>('p')))
        .delivery(DELIVERY)
        .focus_policy(FOCUS)
        .geometry_policy(MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED)
        .source_frame(frame);

    if (!bounded_operation(api, UINT64_C(2000000000), operation)) {
        return false;
    }
    const auto sent = session.send_input(request, operation);
    if (!sent) {
        return report_failure("Session::send_input", sent.error());
    }
    const auto& receipt = sent.value();
    std::printf("receipt: outcome %d delivered %u failure %d cleanup %d\n",
                static_cast<int>(receipt.outcome),
                static_cast<unsigned>(receipt.delivered),
                receipt.failure ? static_cast<int>(receipt.failure->fault) : 0,
                receipt.failure ? static_cast<int>(receipt.failure->cleanup) : 0);
    return expect(receipt.outcome == MADOPILOT_SEQUENCE_COMPLETE &&
                      receipt.delivered == 6,
                  "the bounded native sequence completed exactly once");
}

bool run_native(const madopilot::Api& api, madopilot::Engine& engine,
                std::string_view title, bool check_only)
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
    madopilot::TargetCapability target_capability;
    if (!select_target(targets, title, selected, target_capability)) {
        return false;
    }

    const auto live_descriptor = engine.input_descriptor(targets, selected, operation);
    if (!live_descriptor) {
        return report_failure("Engine::input_descriptor", live_descriptor.error());
    }
    if (!expect((live_descriptor.value().pairs & REQUIRED_PAIRS) == REQUIRED_PAIRS,
                "the live input descriptor retains every required pair")) {
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

    const bool exercised = exercise_session(api, session);
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
    if (argc == 2 && std::string_view(argv[1]) == "--load-check") {
        load_only = true;
    } else if (argc == 2 && std::string_view(argv[1]) == "--check") {
        check_only = true;
    } else if (argc == 2 && argv[1][0] != '\0') {
        title = argv[1];
    } else {
        std::fprintf(stderr,
                     "usage: %s --load-check | --check | \"<full fixture window title>\"\n",
                     argv[0]);
        return 2;
    }

    auto loaded = madopilot::Api::load();
    if (!loaded) {
        report_failure("madopilot::Api::load", loaded.error());
        return 1;
    }
    const madopilot::Api api = loaded.take();
    if (!expect(api.extent() >= MADOPILOT_API_SIZE_SESSION_SEND_INPUT,
                "the negotiated table contains the complete ABI 1.1 suffix")) {
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
    auto created = api.create_engine(source, operation);
    if (!created) {
        report_failure("Api::create_engine", created.error());
        return 1;
    }
    madopilot::Engine engine = created.take();

    if (!run_native(api, engine, title, check_only) || failures != 0) {
        return 1;
    }
    report_peak_resident_bytes();

    std::printf("%s complete%s\n", EXAMPLE_NAME,
                check_only ? " (non-prompting check)" : "");
    return 0;
}
