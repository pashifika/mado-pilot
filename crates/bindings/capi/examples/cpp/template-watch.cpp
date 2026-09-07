/*
 * Pull-based template watching through the public C++17 wrapper and C ABI only.
 * Uses the repository-owned Phase 1 scene and fixtures/assets/phase1-slice.
 * No native permissions, callbacks, input, or caller frame-polling loop.
 *
 * usage: madopilot-cpp-template-watch --package <dir>
 */

#include <cmath>
#include <cstdio>
#include <cstring>
#include <string_view>
#include <vector>

#include "deterministic-scene.h"
#include "madopilot/madopilot.hpp"

namespace {

template <class T>
bool require(const madopilot::Result<T>& result, const char* operation)
{
    if (!result) {
        std::fprintf(stderr, "%s failed: status %d category %d: %s\n", operation,
                     static_cast<int>(result.status()),
                     static_cast<int>(result.error().category()),
                     result.error().message().c_str());
        return false;
    }
    return true;
}

bool expect(bool condition, const char* contract)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", contract);
    }
    return condition;
}

bool same_stamp(const madopilot::FrameStamp& left, const madopilot::FrameStamp& right)
{
    return left.stream == right.stream && left.epoch == right.epoch &&
           left.sequence == right.sequence && left.geometry == right.geometry;
}

} // namespace

int main(int argc, char** argv)
{
    if (argc != 3 || std::string_view(argv[1]) != "--package") {
        std::fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
        return 2;
    }
    auto loaded = madopilot::Api::load();
    if (!require(loaded, "Api::load")) {
        return 1;
    }
    const auto api = loaded.take();
    const auto now = api.clock_now();
    if (!require(now, "clock_now")) {
        return 1;
    }
    madopilot::Operation operation;
    operation.deadline(now.value() + 30ull * 1000ull * 1000ull * 1000ull);

    std::vector<std::uint8_t> scene(SCENE_BYTES);
    scene_fill_rgba(scene.data());
    madopilot::ReplayFrame supplied;
    supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(scene.data(), scene.size());
    auto source = madopilot::Source::replay_memory("template-watch-panel");
    source.frame(supplied);
    auto built = api.create_engine(source, operation);
    if (!require(built, "create_engine")) {
        return 1;
    }
    auto engine = built.take();
    const auto scheduler = engine.template_scheduler_descriptor();
    if (!require(scheduler, "template_scheduler_descriptor")) {
        return 1;
    }
    std::printf("scheduler: engine queries %u session queries %u in-flight %u\n",
                scheduler.value().max_engine_queries,
                scheduler.value().max_session_queries,
                scheduler.value().max_in_flight_analyses);

    auto discovered = engine.discover(operation);
    if (!require(discovered, "discover")) {
        return 1;
    }
    auto targets = discovered.take();
    madopilot::OpenRequest open;
    open.require_format(MADOPILOT_PIXEL_FORMAT_RGBA8);
    auto opened = engine.open_session(targets, 0, open, operation);
    if (!require(opened, "open_session")) {
        return 1;
    }
    auto session = opened.take();
    targets.reset();
    const auto session_info = session.describe();
    if (!require(session_info, "session describe")) {
        return 1;
    }
    auto loaded_package = engine.load_package(
        madopilot::PackageSource::directory(argv[2]), operation);
    if (!require(loaded_package, "load_package")) {
        return 1;
    }
    auto package = loaded_package.take();
    auto prepared = engine.prepare_from_package(package, "panel.patch", operation);
    if (!require(prepared, "prepare_from_package")) {
        return 1;
    }
    auto patch = prepared.take();
    madopilot::TemplateWatchOptions options;
    options.immediate().minimum_interval(0)
        .change_policy(MADOPILOT_TEMPLATE_CHANGE_EXACT_RGBA);
    auto started = session.start_template_watch(patch, options, operation);
    if (!require(started, "start_template_watch")) {
        return 1;
    }
    auto query = started.take();
    patch.reset();
    package.reset();

    // Poll facts are values. A concurrent completion may already supply a
    // terminal owner; no caller-owned buffer or request storage survives start.
    auto polled = query.poll();
    if (!require(polled, "template query poll")) {
        return 1;
    }
    auto observation = polled.take();
    std::printf("query: %llu state %d\n",
                static_cast<unsigned long long>(observation.snapshot.query_id),
                static_cast<int>(observation.snapshot.state));
    if (!expect(observation.pending() ? !observation.terminal
                                     : observation.terminal.has_value(),
                "poll state agrees with its optional owned terminal")) {
        return 1;
    }

    // This is independent caller-wait authority, not a replacement query deadline.
    madopilot::Operation wait_operation;
    wait_operation.deadline(now.value() + 30ull * 1000ull * 1000ull * 1000ull);
    auto waited = query.wait(wait_operation);
    if (!require(waited, "template query wait")) {
        return 1;
    }
    auto result = waited.take();
    const auto info = result.describe();
    if (!require(info, "terminal describe")) {
        return 1;
    }
    if (info.value().outcome != MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED) {
        std::fprintf(stderr, "query terminal: outcome %d status %d\n",
                     static_cast<int>(info.value().outcome),
                     static_cast<int>(info.value().status));
        const auto failure = result.error();
        if (require(failure, "terminal error") && failure.value()) {
            std::fprintf(stderr, "%s\n", failure.value()->message().c_str());
        }
        return 1;
    }
    if (!expect(info.value().status == MADOPILOT_STATUS_OK &&
                    info.value().match_count == 2 &&
                    info.value().target == session_info.value().target &&
                    info.value().template_id.view() == "panel.patch" &&
                    info.value().transform.geometry == info.value().source.geometry &&
                    info.value().transform.width == SCENE_WIDTH &&
                    info.value().transform.height == SCENE_HEIGHT,
                "matched identity, count and transform describe the exact source")) {
        return 1;
    }
    bool planted[2] = {false, false};
    for (std::size_t index = 0; index < 2; ++index) {
        const auto match = result.match_at(index);
        if (!require(match, "match_at")) {
            return 1;
        }
        bool recognized = false;
        for (std::size_t location = 0; location < 2; ++location) {
            if (match.value().bounds.left == static_cast<std::int32_t>(SCENE_PLANTED[location][0]) &&
                match.value().bounds.top == static_cast<std::int32_t>(SCENE_PLANTED[location][1]) &&
                match.value().bounds.right == static_cast<std::int32_t>(SCENE_PLANTED[location][0] + PATCH_WIDTH) &&
                match.value().bounds.bottom == static_cast<std::int32_t>(SCENE_PLANTED[location][1] + PATCH_HEIGHT)) {
                recognized = !planted[location];
                planted[location] = true;
            }
        }
        if (!expect(recognized && std::abs(match.value().score - 1.0) <= 1e-5 &&
                        match.value().template_id.view() == "panel.patch",
                    "matches equal the unordered planted coordinate set")) {
            return 1;
        }
    }
    const auto source_stamp = info.value().source;
    auto retained_frame = result.frame();
    if (!require(retained_frame, "terminal frame")) {
        return 1;
    }
    auto frame = retained_frame.take();
    const auto frame_stamp = frame.stamp();
    if (!require(frame_stamp, "frame stamp") ||
        !expect(same_stamp(frame_stamp.value(), source_stamp),
                "all four result and frame source identities agree")) {
        return 1;
    }
    auto mapped = frame.map(madopilot::MapRequest(), operation);
    if (!require(mapped, "frame map")) {
        return 1;
    }
    auto mapping = mapped.take();
    if (!require(session.close(operation), "session close")) {
        return 1;
    }
    query.reset();
    observation.terminal.reset();
    session.reset();
    engine.reset();
    const auto retained_info = result.describe();
    if (!require(retained_info, "terminal after parent release") ||
        !expect(retained_info.value().template_id.view() == "panel.patch" &&
                    same_stamp(retained_info.value().source, source_stamp),
                "retained terminal facts outlive query, session and engine")) {
        return 1;
    }
    result.reset();
    frame.reset();
    const auto image = mapping.describe();
    const auto mapping_stamp = mapping.stamp();
    if (!require(image, "mapping after parent release") ||
        !require(mapping_stamp, "mapping stamp") ||
        !expect(image.value().bytes.size() == scene.size() &&
                    std::memcmp(image.value().bytes.data(), scene.data(), scene.size()) == 0 &&
                    same_stamp(mapping_stamp.value(), source_stamp),
                "retained mapping remains readable with exact pixels and source")) {
        return 1;
    }
    std::printf("retained: stream %llu epoch %llu sequence %llu geometry %llu bytes %zu\n",
                static_cast<unsigned long long>(source_stamp.stream),
                static_cast<unsigned long long>(source_stamp.epoch),
                static_cast<unsigned long long>(source_stamp.sequence),
                static_cast<unsigned long long>(source_stamp.geometry),
                image.value().bytes.size());
    std::printf("madopilot-cpp-template-watch complete\n");
    return 0;
}
