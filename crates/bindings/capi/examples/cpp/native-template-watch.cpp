/*
 * Native F1-F9 qualification through the public C++17 owners only.
 *
 * --check never discovers a target, requests permission, or claims native PASS.
 * --native --title <exact title> requires the repository's owned-fixture
 * controller on stdin/stdout. Only that controller mutates the fixture. Titles,
 * package paths, errors, and pixels are not written to the qualification ledger.
 */

#include <array>
#include <clocale>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <limits>
#include <string_view>
#include <utility>

#include "madopilot/madopilot.hpp"
#include "../common/native-watch-protocol.h"

namespace {

constexpr std::uint64_t operation_nanos = UINT64_C(5000000000);
constexpr std::uint64_t observation_nanos = UINT64_C(25000000);
constexpr double marker_min_score = 0.95;
constexpr const char* rows[] = {"F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9"};

unsigned long long number(std::uint64_t value)
{
    return static_cast<unsigned long long>(value);
}

madopilot::Source native_source()
{
#if defined(_WIN32)
    return madopilot::Source::native_windows();
#else
    return madopilot::Source::native_macos();
#endif
}

bool same_stamp(const madopilot::FrameStamp& a, const madopilot::FrameStamp& b)
{
    return a.stream == b.stream && a.epoch == b.epoch &&
           a.sequence == b.sequence && a.geometry == b.geometry;
}

bool same_geometry(const madopilot::FrameStamp& a, const madopilot::FrameStamp& b)
{
    return a.stream == b.stream && a.epoch == b.epoch && a.geometry == b.geometry;
}

bool pending_ready(const madopilot::TemplateQuerySnapshot& snapshot,
                   const madopilot::FrameStamp& seen, std::uint64_t completed_after,
                   std::uint64_t generation_after)
{
    return snapshot.completed > completed_after && snapshot.generation > generation_after &&
           (snapshot.flags & MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME) != 0 &&
           same_geometry(snapshot.last_frame, seen) && snapshot.last_frame.sequence >= seen.sequence;
}

bool same_rect(const madopilot::Rect& a, const madopilot::Rect& b)
{
    return a.space == b.space && a.left == b.left && a.top == b.top &&
           a.right == b.right && a.bottom == b.bottom;
}

madopilot::Rect full_frame(std::uint32_t width, std::uint32_t height)
{
    return {MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0,
            static_cast<std::int32_t>(width), static_cast<std::int32_t>(height)};
}

bool same_transform(const madopilot::TransformSnapshot& a,
                    const madopilot::TransformSnapshot& b)
{
    return a.flags == b.flags && a.geometry == b.geometry &&
           a.width == b.width && a.height == b.height &&
           a.desktop_origin_x == b.desktop_origin_x &&
           a.desktop_origin_y == b.desktop_origin_y &&
           a.logical_width == b.logical_width && a.logical_height == b.logical_height &&
           a.target_scale_x == b.target_scale_x && a.target_scale_y == b.target_scale_y &&
           a.desktop_scale_x == b.desktop_scale_x && a.desktop_scale_y == b.desktop_scale_y;
}

bool valid_transform(const madopilot::TransformSnapshot& t)
{
    const auto required = MADOPILOT_TRANSFORM_COVERS_TARGET |
                          MADOPILOT_TRANSFORM_HAS_TARGET_PLACEMENT;
    return (t.flags & required) == required && t.width >= 3 && t.height >= 2 &&
           t.width <= static_cast<std::uint32_t>(INT32_MAX) &&
           t.height <= static_cast<std::uint32_t>(INT32_MAX) &&
           std::isfinite(t.desktop_origin_x) && std::isfinite(t.desktop_origin_y) &&
           std::isfinite(t.logical_width) && t.logical_width > 0 &&
           std::isfinite(t.logical_height) && t.logical_height > 0 &&
           std::isfinite(t.target_scale_x) && t.target_scale_x > 0 &&
           std::isfinite(t.target_scale_y) && t.target_scale_y > 0 &&
           std::isfinite(t.desktop_scale_x) && t.desktop_scale_x > 0 &&
           std::isfinite(t.desktop_scale_y) && t.desktop_scale_y > 0;
}

bool fixed_scheduler(const madopilot::TemplateSchedulerDescriptor& d)
{
    return d.flags == 0 && d.max_engine_queries == 256 && d.max_active_sessions == 16 &&
           d.max_session_queries == 64 && d.max_in_flight_analyses == 2 &&
           d.latest_pending_frames_per_query == 1 && d.max_mapped_cache_entries == 256 &&
           d.mapped_cache_bytes == UINT64_C(67108864) &&
           d.eligible_queue_expiry_nanos == UINT64_C(30000000000);
}

struct Geometry {
    madopilot::FrameStamp source{};
    madopilot::TransformSnapshot transform{};
    mpw_shape shape{};
};

struct Retained {
    madopilot::TemplateQueryResult result;
    madopilot::Frame frame;
    madopilot::Mapping mapping;
    // These views deliberately remain borrowed. Their owners are above.
    madopilot::TemplateQueryResultInfo info;
    madopilot::Match match;
    madopilot::Image image;
    Geometry geometry;
    mpw_token token{};

    void reset()
    {
        info = {};
        match = {};
        image = {};
        result.reset();
        frame.reset();
        mapping.reset();
    }
};

class Qualification {
public:
    Qualification(madopilot::Api api, std::string_view title) : api_(api), title_(title) {}

    int run()
    {
        try {
            for (std::size_t row = 0; row < 8; ++row) {
                if (!step(row)) break;
            }
        } catch (...) {
            // Wrapper allocation failures are not product statuses. Do not leak
            // an exception's text or skip explicit native-session cleanup.
            if (protocol_ok_ && !reported_[current_]) {
                report(current_, "FAIL", "consumer_exception");
            }
        }
        return finish();
    }

private:
    bool fail(const char* reason, madopilot::Status status = MADOPILOT_STATUS_OK,
              const char* outcome = "FAIL")
    {
        if (reason_ == nullptr) {
            reason_ = reason;
            status_ = status;
            outcome_ = outcome;
        }
        return false;
    }

    template <class T>
    bool accept(const madopilot::Result<T>& result, const char* reason)
    {
        return result ? true : fail(reason, result.status());
    }

    bool expect(bool condition, const char* reason)
    {
        return condition || fail(reason);
    }

    bool now(std::uint64_t& value)
    {
        const auto read = api_.clock_now();
        if (!accept(read, "clock_failed")) return false;
        value = read.value();
        return true;
    }

    bool operation(madopilot::Operation& out, std::uint64_t duration = operation_nanos)
    {
        std::uint64_t instant = 0;
        if (!now(instant)) return false;
        if (duration > std::numeric_limits<std::uint64_t>::max() - instant) {
            return fail("deadline_overflow");
        }
        out.deadline(instant + duration);
        return true;
    }

    bool limit(std::uint64_t& until)
    {
        if (!now(until)) return false;
        if (until > std::numeric_limits<std::uint64_t>::max() - operation_nanos) {
            return fail("deadline_overflow");
        }
        until += operation_nanos;
        return true;
    }

    bool slice(std::uint64_t until, madopilot::Operation& out)
    {
        std::uint64_t instant = 0;
        if (!now(instant)) return false;
        if (instant >= until) return fail("observation_deadline");
        const auto remaining = until - instant;
        out.deadline(instant + (remaining < observation_nanos ? remaining : observation_nanos));
        return true;
    }

    template <class... Args>
    bool fact(const char* key, const char* format, Args... args)
    {
        char values[512];
        const int count = std::snprintf(values, sizeof(values), format, args...);
        if (count < 0 || static_cast<std::size_t>(count) >= sizeof(values) ||
            !protocol_ok_ || !mpw_fact(rows[current_], key, values)) {
            protocol_ok_ = false;
            return fail("protocol_failure");
        }
        return true;
    }

    bool frame_fact(const madopilot::FrameStamp& stamp, const madopilot::TransformSnapshot& t)
    {
        return fact("frame", "%llu %llu %llu %llu %u %u", number(stamp.stream),
                    number(stamp.epoch), number(stamp.sequence), number(stamp.geometry),
                    t.width, t.height);
    }

    bool transform_fact(const madopilot::TransformSnapshot& t, std::uint64_t target)
    {
        return fact("transform", "%llu %.17g %.17g %.17g %.17g %.17g %.17g %llu",
                    number(t.geometry), t.desktop_origin_x, t.desktop_origin_y,
                    t.logical_width, t.logical_height, t.target_scale_x, t.target_scale_y,
                    number(target));
    }

    bool backend_version(madopilot::BorrowedStr version)
    {
        const auto value = version.view();
        if (!expect(value.size() > 2 && value.size() < backend_version_.size() &&
                        value.substr(0, 2) == "4.", "backend_version_invalid")) return false;
        if (backend_version_size_ == 0) {
            std::memcpy(backend_version_.data(), value.data(), value.size());
            backend_version_size_ = value.size();
        }
        return expect(value == std::string_view(backend_version_.data(), backend_version_size_),
                      "backend_version_changed");
    }

    bool geometry_facts(const Geometry& g)
    {
        const auto& t = g.transform;
        return frame_fact(seen_, t) && transform_fact(t, target_);
    }

    bool report(std::size_t row, const char* outcome, const char* reason)
    {
        if (reported_[row] || !protocol_ok_ || !mpw_row(rows[row], outcome, reason)) {
            protocol_ok_ = false;
            return false;
        }
        reported_[row] = true;
        return true;
    }

    bool step(std::size_t row)
    {
        current_ = row;
        reason_ = nullptr;
        outcome_ = "PASS";
        status_ = MADOPILOT_STATUS_OK;
        bool success = false;
        switch (row) {
        case 0: success = admission(); break;
        case 1: success = readiness(); break;
        case 2: success = first_match(); break;
        case 3: success = resize(); break;
        case 4: success = movement(); break;
        case 5: success = independent_authority(); break;
        case 6: success = retained_ownership(); break;
        case 7: success = lifecycle_terminals(); break;
        default: return fail("contract_mismatch");
        }
        if (!success) {
            if (protocol_ok_) fact("status", "%d %d", static_cast<int>(status_),
                                    static_cast<int>(MADOPILOT_TEMPLATE_QUERY_OUTCOME_NONE));
            report(row, outcome_, reason_ == nullptr ? "contract_mismatch" : reason_);
            return false;
        }
        return report(row, outcome_, reason_ == nullptr ? pass_reason(row) : reason_);
    }

    static const char* pass_reason(std::size_t row)
    {
        constexpr const char* reasons[] = {
            "admission_observed", "absent_completed_pending", "exact_match_correlated",
            "resize_correlated", "movement_correlated", "independent_authority",
            "retained_ownership", "lifecycle_terminals", "consumer_cleanup"
        };
        return reasons[row];
    }

    bool open_native()
    {
        madopilot::Operation op;
        if (!operation(op)) return false;
        auto created = api_.create_engine(native_source(), op);
        if (!created) {
            return fail("native_engine_refused", created.status(),
                        created.status() == MADOPILOT_STATUS_UNSUPPORTED ? "UNSUPPORTED" : "FAIL");
        }
        engine_ = created.take();
        const auto capabilities = engine_.capabilities();
        const auto scheduler = engine_.template_scheduler_descriptor();
        if (!accept(capabilities, "capabilities_failed") ||
            !accept(scheduler, "scheduler_descriptor_failed") ||
            !expect(fixed_scheduler(scheduler.value()), "scheduler_contract_mismatch")) return false;
#if defined(__APPLE__)
        constexpr bool reads_permissions = true;
#else
        constexpr bool reads_permissions = false;
#endif
        if (!expect(capabilities.value().reads_permissions() == reads_permissions,
                    "permission_capability_mismatch") ||
            !expect(capabilities.value().delivers_input(), "native_capability_mismatch")) return false;
        std::array<madopilot::PermissionState, 2> states = {
            MADOPILOT_PERMISSION_STATE_UNAVAILABLE, MADOPILOT_PERMISSION_STATE_UNAVAILABLE
        };
        constexpr madopilot::PermissionKind kinds[] = {
            MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE, MADOPILOT_PERMISSION_KIND_INPUT_CONTROL
        };
        for (std::size_t index = 0; index < states.size(); ++index) {
            if (!operation(op)) return false;
            const auto permission = engine_.permission(kinds[index], op);
            if (reads_permissions) {
                if (!accept(permission, "permission_probe_failed") ||
                    !expect(permission.value().kind == kinds[index], "permission_kind_mismatch")) return false;
                states[index] = permission.value().state;
            } else if (!expect(!permission && permission.status() == MADOPILOT_STATUS_UNSUPPORTED,
                               "permission_refusal_mismatch")) return false;
        }
        if (!fact("permissions", "%d %d", static_cast<int>(states[0]), static_cast<int>(states[1]))) return false;
        // Input authority is recorded separately and is never required or used.
        if (reads_permissions && states[0] != MADOPILOT_PERMISSION_STATE_GRANTED) {
            return fail("capture_permission_not_granted", MADOPILOT_STATUS_OK, "UNEXECUTED");
        }
        if (!operation(op)) return false;
        auto discovered = engine_.discover(op);
        if (!accept(discovered, "native_discovery_refused")) return false;
        auto targets = discovered.take();
        const auto count = targets.count();
        if (!accept(count, "target_count_failed")) return false;
        std::size_t selected = 0;
        std::size_t matches = 0;
        madopilot::TargetDescriptor candidate;
        for (std::size_t index = 0; index < count.value(); ++index) {
            const auto descriptor = targets.at(index);
            if (!accept(descriptor, "target_descriptor_failed")) return false;
            if (descriptor.value().has_kind() &&
                descriptor.value().kind == MADOPILOT_TARGET_KIND_WINDOW &&
                descriptor.value().name.view() == title_) {
                selected = index;
                candidate = descriptor.value();
                ++matches;
            }
        }
        if (matches == 0) return fail("owned_title_not_discovered", MADOPILOT_STATUS_OK, "UNEXECUTED");
        if (!expect(matches == 1, "owned_title_not_unique") ||
            !expect(candidate.target != 0 && candidate.supports_space(MADOPILOT_SPACE_CAPTURE_PIXELS),
                    "target_capability_mismatch")) return false;
        if (candidate.capture == MADOPILOT_CAPABILITY_UNSUPPORTED) {
            return fail("target_capture_unsupported", MADOPILOT_STATUS_OK, "UNSUPPORTED");
        }
        target_ = candidate.target;
        madopilot::OpenRequest request; // Capture-only; do not require native RGBA storage.
        if (!operation(op)) return false;
        auto opened = engine_.open_session(targets, selected, request, op);
        if (!opened) {
            return fail("native_session_refused", opened.status(),
                        opened.status() == MADOPILOT_STATUS_UNSUPPORTED ? "UNSUPPORTED" : "FAIL");
        }
        session_ = opened.take();
        if (!now(opened_at_)) return false;
        targets.reset(); // No hidden engine reference may survive F8's final engine release.
        const auto info = session_.describe();
        if (!accept(info, "session_describe_failed") ||
            !expect(info.value().target == target_ && info.value().stream != 0 &&
                        info.value().accepts_input == 0, "session_target_mismatch")) return false;
        session_stream_ = info.value().stream;
        return true;
    }

    bool resource_sample(unsigned phase, unsigned index)
    {
        mpw_resource_snapshot snapshot{};
        if (!mpw_resources(&snapshot)) {
            return fail("resource_snapshot_failed", MADOPILOT_STATUS_OK, "INFRA");
        }
        return fact("resource", "%u %u %llu %llu %llu", phase, index,
                    number(snapshot.private_or_footprint_bytes), number(snapshot.resident_bytes),
                    number(snapshot.handle_or_port_count));
    }

    bool resource_lifetime(bool fresh_session)
    {
        if (fresh_session && !open_native()) return false;
        mpw_token absent{};
        madopilot::TemplateQuerySnapshot pending_snapshot{};
        Retained matched;
        if (!absent_setup(geometry_, absent, seen_) ||
            !start_pending(geometry_, seen_, query_, pending_snapshot) ||
            !visible_match(query_, geometry_, matched)) return false;
        query_.reset();
        matched.reset();
        if (!close_session()) return false;
        session_.reset();
        engine_.reset();
        return expect(query_.empty() && matched.result.empty() && matched.frame.empty() &&
                          matched.mapping.empty() && session_.empty() && engine_.empty(),
                      "consumer_cleanup_failed");
    }

    bool resource_prelude()
    {
        // Exactly one warmup, then three measured lifetimes in this process,
        // fixture and loaded module. No owner is retained at a sample boundary.
        // Numeric observations remain under F1 and never qualify F3 or F7.
        if (!resource_lifetime(false) || !resource_sample(0, 0)) return false;
        for (unsigned index = 1; index <= 3; ++index) {
            if (!resource_lifetime(true) || !resource_sample(1, index)) return false;
        }
        return true;
    }

    bool admission()
    {
#if !defined(__APPLE__) && !defined(_WIN32)
        return fail("native_platform_unavailable", MADOPILOT_STATUS_UNSUPPORTED, "UNSUPPORTED");
#else
        const auto build = api_.describe_build();
        if (!accept(build, "build_descriptor_failed") ||
            !expect(build.value().abi_major == 1 && build.value().abi_minor >= 6 &&
                        build.value().table_size >= MADOPILOT_API_SIZE_ABI_1_6 &&
                        api_.extent() >= MADOPILOT_API_SIZE_TEMPLATE_QUERY_REQUIRED &&
                        api_.extent() >= MADOPILOT_API_SIZE_TEMPLATE_QUERY_RESULT_REQUIRED,
                    "abi_extent_mismatch")) return false;
        // start_template_watch and every result accessor use the wrapper's
        // complete transitive lifecycle guards; no C table entry is invoked here.
        return open_native() && resource_prelude() && open_native() &&
               fact("status", "%d %d", static_cast<int>(MADOPILOT_STATUS_OK),
                    static_cast<int>(MADOPILOT_TEMPLATE_QUERY_OUTCOME_NONE));
#endif
    }

    bool start(std::uint32_t cell_w, std::uint32_t cell_h, double min_score,
               std::uint64_t lifetime, madopilot::TemplateQuery& out)
    {
        char path[2048];
        if (!mpw_asset(cell_w, cell_h, path, sizeof(path))) return fail("asset_protocol_failure");
        madopilot::Operation op;
        if (!operation(op)) return false;
        auto loaded = engine_.load_package(madopilot::PackageSource::directory(path), op);
        if (!accept(loaded, "package_load_failed")) return false;
        auto package = loaded.take();
        if (!operation(op)) return false;
        auto prepared = engine_.prepare_from_package(package, "native.marker", op);
        if (!accept(prepared, "template_prepare_failed")) return false;
        auto marker = prepared.take();
        const auto description = marker.describe();
        if (!accept(description, "template_describe_failed") ||
            !expect(description.value().id.view() == "native.marker" &&
                        description.value().backend.view() == "opencv-cpu" &&
                        description.value().width == 3u * cell_w &&
                        description.value().height == 2u * cell_h,
                    "prepared_asset_mismatch")) return false;
        madopilot::MatchOptions match;
        match.min_score(min_score).max_results(1).suppression(MADOPILOT_SUPPRESSION_DROP_OVERLAPPING);
        madopilot::TemplateWatchOptions options;
        options.match_options(match).full_frame().immediate().minimum_interval(0)
            .change_policy(min_score == 0 ? MADOPILOT_TEMPLATE_CHANGE_ANALYSIS_ALWAYS
                                         : MADOPILOT_TEMPLATE_CHANGE_EXACT_RGBA);
        if (!operation(op, lifetime)) return false;
        auto started = session_.start_template_watch(marker, options, op);
        if (!accept(started, "query_start_failed")) return false;
        out = started.take();
        // This helper's package source, options, operation and string storage
        // also disappear on return. Only the public query retains its authority.
        marker.reset();
        package.reset();
        return expect(static_cast<bool>(out), "query_owner_missing");
    }

    bool wait(madopilot::TemplateQuery& query, madopilot::TemplateQueryResult& out)
    {
        madopilot::Operation op;
        if (!operation(op)) return false;
        auto waited = query.wait(op);
        if (!accept(waited, "query_wait_failed")) return false;
        out = waited.take();
        return true;
    }

    bool bootstrap(Geometry& out, const madopilot::FrameStamp* changed_from = nullptr)
    {
        // ABI 1.6 exposes frame-time placement only on a Matched watch result.
        // This single permissive 3x2 probe supplies metadata, NEVER native match
        // qualification or stability. There is no fallback or replacement probe.
        madopilot::TemplateQuery probe;
        madopilot::TemplateQueryResult result;
        if (!start(1, 1, 0, operation_nanos, probe) || !wait(probe, result)) return false;
        const auto info = result.describe();
        if (!accept(info, "bootstrap_result_failed")) return false;
        if (info.value().outcome != MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED) {
            if (!fact("status", "%d %d", static_cast<int>(MADOPILOT_STATUS_OK),
                      static_cast<int>(info.value().outcome))) return false;
            return fail("bootstrap_metadata_unavailable");
        }
        if (!expect(info.value().status == MADOPILOT_STATUS_OK && info.value().target == target_ &&
                        info.value().match_count == 1 && valid_transform(info.value().transform) &&
                        info.value().source.geometry == info.value().transform.geometry,
                    "bootstrap_metadata_unavailable")) return false;
        if (!backend_version(info.value().backend_version) ||
            !expect(info.value().source.stream == session_stream_, "bootstrap_session_mismatch")) return false;
        auto exact = result.frame();
        if (!accept(exact, "bootstrap_frame_failed")) return false;
        auto frame = exact.take();
        const auto stamp = frame.stamp();
        const auto description = frame.describe();
        if (!accept(stamp, "bootstrap_stamp_failed") || !accept(description, "bootstrap_frame_info_failed") ||
            !expect(same_stamp(stamp.value(), info.value().source) &&
                        description.value().width == info.value().transform.width &&
                        description.value().height == info.value().transform.height,
                    "bootstrap_frame_mismatch")) return false;
        if (changed_from != nullptr &&
            !expect(stamp.value().stream == changed_from->stream &&
                        !same_geometry(stamp.value(), *changed_from), "bootstrap_geometry_stale")) return false;
        out.source = stamp.value();
        out.transform = info.value().transform;
        frame.reset();
        result.reset();
        probe.reset();
        const auto& t = out.transform;
        if (!mpw_geometry(t.width, t.height, t.desktop_origin_x, t.desktop_origin_y,
                          t.logical_width, t.logical_height, t.target_scale_x, t.target_scale_y,
                          &out.shape)) return fail("geometry_protocol_failure");
        return fact("bootstrap", "%llu %llu %llu %llu %u %u", number(out.source.stream),
                    number(out.source.epoch), number(out.source.sequence), number(out.source.geometry),
                    t.width, t.height) && transform_fact(t, target_);
    }

    bool map_frame(const madopilot::Frame& frame, const Geometry& g,
                   const madopilot::Operation& op, madopilot::Mapping& mapping,
                   madopilot::Image& image)
    {
        const auto stamp = frame.stamp();
        const auto description = frame.describe();
        if (!accept(stamp, "frame_stamp_failed") || !accept(description, "frame_describe_failed") ||
            !expect(same_geometry(stamp.value(), g.source) &&
                        description.value().width == g.transform.width &&
                        description.value().height == g.transform.height &&
                        description.value().space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
                        same_rect(description.value().bounds, full_frame(g.transform.width, g.transform.height)),
                    "frame_geometry_mismatch")) return false;
        auto mapped = frame.map(madopilot::MapRequest(), op);
        if (!accept(mapped, "frame_map_failed")) return false;
        mapping = mapped.take();
        const auto described = mapping.describe();
        const auto mapped_stamp = mapping.stamp();
        if (!accept(described, "mapping_describe_failed") || !accept(mapped_stamp, "mapping_stamp_failed") ||
            !expect(same_stamp(stamp.value(), mapped_stamp.value()) &&
                        described.value().format == MADOPILOT_PIXEL_FORMAT_RGBA8 &&
                        described.value().space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
                        described.value().width == g.transform.width &&
                        described.value().height == g.transform.height &&
                        same_rect(described.value().region, full_frame(g.transform.width, g.transform.height)),
                    "mapping_correlation_mismatch")) return false;
        image = described.value();
        return true;
    }

    static bool token_matches(const madopilot::Image& image, const Geometry& g, const mpw_token& token)
    {
        return mpw_pixels_match_token(image.bytes.data(), image.bytes.size(), image.stride,
                                      image.width, image.height, &g.shape, &token) == 1;
    }

    bool visual(bool visible, mpw_token& token)
    {
        return mpw_visual(visible ? 1 : 0, &token) ? true : fail("visual_protocol_failure");
    }

    bool observe(const Geometry& g, const mpw_token& token, madopilot::FrameStamp& seen)
    {
        std::uint64_t until = 0;
        if (!limit(until)) return false;
        for (;;) {
            madopilot::Operation op;
            if (!slice(until, op)) return false;
            auto acquired = session_.acquire_frame(op);
            if (!acquired && acquired.status() == MADOPILOT_STATUS_DEADLINE_EXCEEDED) {
                mpw_pause();
                continue; // One bounded observation window, not a new stimulus.
            }
            if (!accept(acquired, "token_frame_failed")) return false;
            auto frame = acquired.take();
            madopilot::Mapping mapping;
            madopilot::Image image;
            if (!slice(until, op) || !map_frame(frame, g, op, mapping, image)) return false;
            if (token_matches(image, g, token)) {
                const auto stamp = frame.stamp();
                if (!accept(stamp, "token_stamp_failed")) return false;
                seen = stamp.value();
                if (!now(observed_at_) ||
                    !expect(observed_at_ < until, "observation_deadline")) return false;
                return frame_fact(seen, g.transform) &&
                       fact("mapped_view_bytes", "%llu", number(image.bytes.size()));
            }
            mpw_pause();
        }
    }

    bool pending(madopilot::TemplateQuery& query, const madopilot::FrameStamp& seen,
                 madopilot::TemplateQuerySnapshot& out, std::uint64_t completed_after = 0,
                 std::uint64_t generation_after = 0)
    {
        std::uint64_t until = 0;
        if (!limit(until)) return false;
        for (;;) {
            madopilot::Operation pacing;
            if (!slice(until, pacing)) return false;
            auto polled = query.poll();
            if (!accept(polled, "pending_poll_failed")) return false;
            auto observation = polled.take();
            if (!observation.pending() || observation.terminal) {
                if (observation.terminal) {
                    const auto terminal = observation.terminal->describe();
                    if (!accept(terminal, "terminal_describe_failed") ||
                        !fact("status", "%d %d", static_cast<int>(MADOPILOT_STATUS_OK),
                              static_cast<int>(terminal.value().outcome))) return false;
                }
                return fail("expected_pending_query");
            }
            const auto& s = observation.snapshot;
            if (!expect(s.query_id != 0 && s.pending_count <= 1 && s.in_flight_count <= 1 &&
                            s.confirmed_observations == 0 && s.confirmed_duration_nanos == 0 &&
                            s.failed == 0 && s.queue_expired == 0,
                        "pending_stability_mismatch")) return false;
            if (pending_ready(s, seen, completed_after, generation_after)) {
                out = s;
                return fact("pending", "%llu %u %llu", number(s.completed), s.confirmed_observations,
                            number(s.confirmed_duration_nanos));
            }
            mpw_pause();
        }
    }

    bool absent_setup(Geometry& geometry, mpw_token& token, madopilot::FrameStamp& seen)
    {
        return visual(false, token) && bootstrap(geometry) && observe(geometry, token, seen);
    }

    bool start_pending(const Geometry& geometry, const madopilot::FrameStamp& seen,
                       madopilot::TemplateQuery& query, madopilot::TemplateQuerySnapshot& snapshot,
                       std::uint64_t lifetime = operation_nanos)
    {
        return start(geometry.shape.marker_cell_w, geometry.shape.marker_cell_h,
                     marker_min_score, lifetime, query) && pending(query, seen, snapshot);
    }

    bool readiness()
    {
        mpw_token absent{};
        if (!absent_setup(geometry_, absent, seen_)) return false;
        if (!expect(observed_at_ >= opened_at_, "startup_clock_mismatch") ||
            !fact("startup_nanos", "%llu", number(observed_at_ - opened_at_))) return false;
        madopilot::TemplateQuerySnapshot snapshot{};
        return start_pending(geometry_, seen_, query_, snapshot);
    }

    bool correlate(madopilot::TemplateQueryResult result, const Geometry& geometry,
                   const mpw_token& token, Retained& out)
    {
        const auto info = result.describe();
        if (!accept(info, "match_result_failed")) return false;
        if (info.value().outcome != MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED) {
            if (!fact("status", "%d %d", static_cast<int>(MADOPILOT_STATUS_OK),
                      static_cast<int>(info.value().outcome))) return false;
            return fail("matched_contract_mismatch");
        }
        const auto& r = info.value();
        const auto options = MADOPILOT_MATCH_HAS_MIN_SCORE | MADOPILOT_MATCH_HAS_MAX_RESULTS |
                             MADOPILOT_MATCH_HAS_SUPPRESSION;
        if (!expect(token.visible == 1 && r.outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED &&
                        r.status == MADOPILOT_STATUS_OK && r.overload == MADOPILOT_TEMPLATE_OVERLOAD_NONE &&
                        r.query_id != 0 && r.target == target_ && r.match_count == 1 &&
                        r.template_id.view() == "native.marker" && r.backend_id.view() == "opencv-cpu" &&
                        r.backend_version.view() == std::string_view(backend_version_.data(), backend_version_size_) &&
                        r.options.flags == options && r.options.min_score == marker_min_score &&
                        r.options.max_results == 1 && r.options.suppression == MADOPILOT_SUPPRESSION_DROP_OVERLAPPING &&
                        r.confirmed_observations == 1 && r.confirmed_duration_nanos == 0 &&
                        same_rect(r.effective_region, full_frame(geometry.transform.width, geometry.transform.height)) &&
                        same_geometry(r.source, geometry.source) &&
                        same_transform(r.transform, geometry.transform),
                    "matched_contract_mismatch")) return false;
        const auto match = result.match_at(0);
        const auto invalid = result.match_at(1);
        const auto error = result.error();
        if (!accept(match, "indexed_match_failed") || !accept(error, "matched_error_failed")) return false;
        const auto& shape = geometry.shape;
        const madopilot::Rect expected = {
            MADOPILOT_SPACE_CAPTURE_PIXELS, shape.marker_x, shape.marker_y,
            static_cast<std::int32_t>(static_cast<std::int64_t>(shape.marker_x) + 3ll * shape.marker_cell_w),
            static_cast<std::int32_t>(static_cast<std::int64_t>(shape.marker_y) + 2ll * shape.marker_cell_h)
        };
        if (!expect(same_rect(match.value().bounds, expected) &&
                        match.value().template_id.view() == "native.marker" &&
                        std::isfinite(match.value().score) && match.value().score >= marker_min_score &&
                        match.value().score <= 1 && !error.value() &&
                        !invalid && invalid.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
                    "indexed_match_contract_mismatch")) return false;
        auto exact = result.frame();
        if (!accept(exact, "matched_frame_failed")) return false;
        auto frame = exact.take();
        const auto stamp = frame.stamp();
        if (!accept(stamp, "matched_frame_stamp_failed") ||
            !expect(same_stamp(stamp.value(), r.source), "matched_frame_identity_mismatch")) return false;
        madopilot::Operation op;
        madopilot::Mapping mapping;
        madopilot::Image image;
        if (!operation(op) || !map_frame(frame, geometry, op, mapping, image) ||
            !expect(token_matches(image, geometry, token), "exact_result_token_mismatch")) return false;
        if (!frame_fact(r.source, r.transform) ||
            !transform_fact(r.transform, r.target) ||
            !fact("match", "%llu %llu %llu %u %llu %d %d %d %d", number(r.query_id), number(r.target),
                  number(r.match_count), r.confirmed_observations, number(r.confirmed_duration_nanos),
                  expected.left, expected.top, expected.right, expected.bottom) ||
            !fact("mapped_view_bytes", "%llu", number(image.bytes.size()))) return false;
        out.info = r;
        out.match = match.value();
        out.image = image;
        out.geometry = geometry;
        out.token = token;
        out.result = result.clone();
        if (!expect(static_cast<bool>(out.result), "result_clone_failed")) return false;
        result.reset(); // Borrowed info/indexed views now rely only on the clone.
        out.frame = std::move(frame);
        out.mapping = std::move(mapping);
        return expect(frame.empty() && mapping.empty(), "retained_move_mismatch");
    }

    bool visible_match(madopilot::TemplateQuery& query, const Geometry& geometry, Retained& out)
    {
        auto polled = query.poll();
        if (!accept(polled, "before_visual_poll_failed")) return false;
        auto before = polled.take();
        if (!expect(before.pending() && !before.terminal, "before_visual_not_pending")) return false;
        mpw_token token{};
        madopilot::TemplateQueryResult result;
        if (!visual(true, token) || !wait(query, result)) return false;
        const auto info = result.describe();
        return accept(info, "visible_result_info_failed") &&
               expect(info.value().query_id == before.snapshot.query_id, "visible_query_identity_mismatch") &&
               correlate(std::move(result), geometry, token, out);
    }

    bool first_match()
    {
        if (!visible_match(query_, geometry_, retained_)) return false;
        query_.reset();
        return true;
    }

    bool changed_frame(const Geometry& before, bool resized)
    {
        std::uint64_t until = 0;
        if (!limit(until)) return false;
        for (;;) {
            madopilot::Operation op;
            if (!slice(until, op)) return false;
            auto acquired = session_.acquire_frame(op);
            if (!acquired && acquired.status() == MADOPILOT_STATUS_DEADLINE_EXCEEDED) {
                mpw_pause();
                continue;
            }
            if (!accept(acquired, "geometry_frame_failed")) return false;
            auto frame = acquired.take();
            const auto stamp = frame.stamp();
            const auto info = frame.describe();
            if (!accept(stamp, "geometry_stamp_failed") || !accept(info, "geometry_frame_info_failed") ||
                !expect(stamp.value().stream == before.source.stream, "geometry_stream_replaced")) return false;
            if (!same_geometry(stamp.value(), before.source) &&
                (!resized || info.value().width != before.transform.width ||
                             info.value().height != before.transform.height)) return true;
            mpw_pause();
        }
    }

    bool nonmatched(const madopilot::TemplateQueryResult& result,
                    madopilot::TemplateQueryOutcome outcome, madopilot::Status status,
                    std::uint64_t query_id)
    {
        const auto described = result.describe();
        const auto match = result.match_at(0);
        const auto frame = result.frame();
        const auto error = result.error();
        if (!accept(described, "terminal_describe_failed") || !accept(error, "terminal_error_failed")) return false;
        const auto& r = described.value();
        const auto& t = r.transform;
        // Failed Results expose no value/owner in C++; raw output-buffer reset
        // belongs to the independent C consumer rather than a C-table shortcut.
        return expect(r.outcome == outcome && r.status == status && r.query_id == query_id && query_id != 0 &&
                          r.overload == MADOPILOT_TEMPLATE_OVERLOAD_NONE && r.target == 0 && r.match_count == 0 &&
                          r.confirmed_observations == 0 && r.confirmed_duration_nanos == 0 &&
                          r.template_id.empty() && r.backend_id.empty() && r.backend_version.empty() &&
                          r.source.flags == 0 && r.source.stream == 0 && r.source.epoch == 0 &&
                          r.source.sequence == 0 && r.source.geometry == 0 &&
                          same_rect(r.effective_region, full_frame(0, 0)) &&
                          r.options.flags == 0 && r.options.min_score == 0 && r.options.max_results == 0 &&
                          r.options.suppression == MADOPILOT_SUPPRESSION_DROP_OVERLAPPING &&
                          t.flags == 0 && t.geometry == 0 && t.width == 0 && t.height == 0 &&
                          t.desktop_origin_x == 0 && t.desktop_origin_y == 0 &&
                          t.logical_width == 0 && t.logical_height == 0 &&
                          t.target_scale_x == 0 && t.target_scale_y == 0 &&
                          t.desktop_scale_x == 0 && t.desktop_scale_y == 0 &&
                          !match && match.status() == MADOPILOT_STATUS_INVALID_ARGUMENT &&
                          !frame && frame.status() == MADOPILOT_STATUS_INVALID_ARGUMENT && !error.value(),
                      "nonmatched_terminal_contract_mismatch") &&
               fact("status", "%d %d", static_cast<int>(MADOPILOT_STATUS_OK), static_cast<int>(outcome));
    }

    bool cancelled(madopilot::TemplateQuery& query, std::uint64_t id)
    {
        auto first = query.cancel();
        auto second = query.cancel();
        if (!accept(first, "query_cancel_failed") || !accept(second, "query_repeat_cancel_failed")) return false;
        auto a = first.take();
        auto b = second.take();
        if (!nonmatched(a, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED, MADOPILOT_STATUS_CANCELLED, id) ||
            !nonmatched(b, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED, MADOPILOT_STATUS_CANCELLED, id)) return false;
        auto polled = query.poll();
        if (!accept(polled, "cancelled_poll_failed")) return false;
        auto observed = polled.take();
        return expect(!observed.pending() && observed.snapshot.query_id == id && observed.terminal.has_value(),
                      "cancelled_poll_mismatch") &&
               nonmatched(*observed.terminal, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED,
                          MADOPILOT_STATUS_CANCELLED, id);
    }

    bool transition(const char* action, bool resized)
    {
        mpw_token absent{};
        if (!visual(false, absent) || !observe(geometry_, absent, seen_)) return false;
        madopilot::TemplateQuerySnapshot before_pending{};
        if (!start_pending(geometry_, seen_, query_, before_pending) || !geometry_facts(geometry_)) return false;
        const int changed = mpw_action(action);
        if (changed == -1) {
            if (!cancelled(query_, before_pending.query_id)) return false;
            query_.reset();
            outcome_ = "UNEXECUTED";
            reason_ = resized ? "resize_unavailable" : "movement_topology_unavailable";
            return true;
        }
        if (changed != 1) return fail("geometry_action_failed");
        Geometry next;
        madopilot::FrameStamp new_seen{};
        if (!visual(false, absent) || !changed_frame(geometry_, resized) ||
            !bootstrap(next, &geometry_.source) || !observe(next, absent, new_seen)) return false;
        const auto& old = geometry_.transform;
        const auto& fresh = next.transform;
        if (resized) {
            if (!expect(fresh.width != old.width || fresh.height != old.height,
                        "resize_extent_unchanged")) return false;
        } else {
            if (!expect(fresh.desktop_origin_x != old.desktop_origin_x ||
                            fresh.desktop_origin_y != old.desktop_origin_y,
                        "movement_origin_unchanged")) return false;
#if defined(_WIN32)
            if (!expect(fresh.target_scale_x != old.target_scale_x ||
                            fresh.target_scale_y != old.target_scale_y,
                        "movement_effective_scale_unchanged")) return false;
#endif
        }
        // Observe completed new-absent work on the OLD query before releasing
        // its authority. A status-only cancellation cannot qualify this row.
        madopilot::TemplateQuerySnapshot after_pending{};
        if (!pending(query_, new_seen, after_pending, before_pending.completed, before_pending.generation) ||
            !cancelled(query_, before_pending.query_id)) return false;
        query_.reset();
        geometry_ = next;
        seen_ = new_seen;
        madopilot::TemplateQuerySnapshot replacement{};
        Retained result;
        if (!start_pending(geometry_, seen_, query_, replacement) ||
            !visible_match(query_, geometry_, result)) return false;
        query_.reset();
        return true;
    }

    bool resize() { return transition("RESIZE", true); }
    bool movement() { return transition("MOVE", false); }

    bool independent_authority()
    {
        mpw_token absent{};
        if (!visual(false, absent) || !observe(geometry_, absent, seen_)) return false;
        madopilot::TemplateQuerySnapshot initial{};
        if (!start_pending(geometry_, seen_, query_, initial)) return false;
        {
            auto intermediate = query_.clone();
            if (!expect(static_cast<bool>(intermediate), "query_clone_failed")) return false;
        } // Nonfinal release must not cancel shared native work.
        auto moved = std::move(query_);
        if (!expect(query_.empty() && static_cast<bool>(moved), "query_move_mismatch")) return false;
        const auto empty_poll = query_.poll();
        if (!expect(!empty_poll && empty_poll.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
                    "moved_query_refusal_mismatch")) return false;
        query_ = std::move(moved);
        auto created = api_.create_cancellation();
        if (!accept(created, "caller_cancellation_create_failed")) return false;
        auto cancellation = created.take();
        if (!accept(cancellation.cancel(), "caller_cancellation_failed")) return false;
        madopilot::Operation caller;
        if (!operation(caller)) return false;
        caller.cancellation(cancellation);
        const auto cancelled_wait = query_.wait(caller);
        if (!expect(!cancelled_wait && cancelled_wait.status() == MADOPILOT_STATUS_CANCELLED,
                    "caller_cancel_authority_mismatch") ||
            !fact("status", "%d %d", static_cast<int>(cancelled_wait.status()),
                  static_cast<int>(MADOPILOT_TEMPLATE_QUERY_OUTCOME_NONE))) return false;
        caller.no_cancellation();
        cancellation.reset();
        std::uint64_t instant = 0;
        if (!now(instant)) return false;
        caller.deadline(instant);
        const auto expired_wait = query_.wait(caller);
        if (!expect(!expired_wait && expired_wait.status() == MADOPILOT_STATUS_DEADLINE_EXCEEDED,
                    "caller_deadline_authority_mismatch") ||
            !fact("status", "%d %d", static_cast<int>(expired_wait.status()),
                  static_cast<int>(MADOPILOT_TEMPLATE_QUERY_OUTCOME_NONE))) return false;
        madopilot::TemplateQuerySnapshot still_pending{};
        if (!pending(query_, seen_, still_pending) ||
            !expect(still_pending.query_id == initial.query_id, "caller_wait_replaced_query")) return false;
        Retained success;
        if (!visible_match(query_, geometry_, success)) return false;
        const auto immutable = query_.cancel();
        if (!accept(immutable, "matched_cancel_failed")) return false;
        const auto immutable_info = immutable.value().describe();
        if (!accept(immutable_info, "matched_cancel_info_failed") ||
            !expect(immutable_info.value().outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED &&
                        same_stamp(immutable_info.value().source, success.info.source),
                    "matched_terminal_mutated")) return false;
        query_.reset();
        if (!visual(false, absent) || !observe(geometry_, absent, seen_)) return false;
        madopilot::TemplateQuerySnapshot deadline_pending{};
        if (!start_pending(geometry_, seen_, query_, deadline_pending, UINT64_C(1000000000))) return false;
        madopilot::TemplateQueryResult deadline;
        if (!wait(query_, deadline) ||
            !nonmatched(deadline, MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED,
                        MADOPILOT_STATUS_DEADLINE_EXCEEDED, deadline_pending.query_id)) return false;
        auto repeated = query_.cancel();
        if (!accept(repeated, "expired_query_cancel_failed") ||
            !nonmatched(repeated.value(), MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED,
                        MADOPILOT_STATUS_DEADLINE_EXCEEDED, deadline_pending.query_id)) return false;
        query_.reset();
        madopilot::TemplateQuerySnapshot cancel_pending{};
        if (!start_pending(geometry_, seen_, query_, cancel_pending) ||
            !cancelled(query_, cancel_pending.query_id)) return false;
        query_.reset();
        return true;
    }

    bool close_session()
    {
        if (!session_) return true;
        madopilot::Operation op;
        if (!operation(op) || !accept(session_.close(op), "session_close_failed")) return false;
        if (!operation(op) || !accept(session_.close(op), "session_repeat_close_failed")) return false;
        const auto closed = session_.is_closed();
        return accept(closed, "session_closed_state_failed") &&
               expect(closed.value(), "session_close_not_observed");
    }

    bool verify_retained()
    {
        const auto description = retained_.result.describe();
        const auto match = retained_.result.match_at(0);
        const auto stamp = retained_.frame.stamp();
        const auto mapping_stamp = retained_.mapping.stamp();
        const auto image = retained_.mapping.describe();
        if (!accept(description, "retained_result_failed") || !accept(match, "retained_index_failed") ||
            !accept(stamp, "retained_frame_failed") || !accept(mapping_stamp, "retained_mapping_stamp_failed") ||
            !accept(image, "retained_mapping_failed")) return false;
        return expect(retained_.info.template_id.view() == "native.marker" &&
                          retained_.info.backend_id.view() == "opencv-cpu" &&
                          retained_.match.template_id.view() == "native.marker" &&
                          retained_.info.backend_version.view() ==
                              std::string_view(backend_version_.data(), backend_version_size_) &&
                          retained_.info.backend_version.view() == description.value().backend_version.view() &&
                          same_stamp(description.value().source, retained_.info.source) &&
                          same_stamp(stamp.value(), retained_.info.source) &&
                          same_stamp(mapping_stamp.value(), retained_.info.source) &&
                          same_transform(description.value().transform, retained_.info.transform) &&
                          same_rect(match.value().bounds, retained_.match.bounds) &&
                          match.value().score == retained_.match.score &&
                          token_matches(retained_.image, retained_.geometry, retained_.token) &&
                          token_matches(image.value(), retained_.geometry, retained_.token),
                      "retained_borrowed_views_mismatch");
    }

    bool retained_ownership()
    {
        if (!close_session()) return false;
        query_.reset();
        session_.reset();
        engine_.reset();
        if (!verify_retained()) return false;
        const auto retained_source = retained_.info.source;
        const auto& bounds = retained_.match.bounds;
        if (!frame_fact(retained_source, retained_.info.transform) ||
            !transform_fact(retained_.info.transform, retained_.info.target) ||
            !fact("match", "%llu %llu %llu %u %llu %d %d %d %d",
                  number(retained_.info.query_id), number(retained_.info.target),
                  number(retained_.info.match_count), retained_.info.confirmed_observations,
                  number(retained_.info.confirmed_duration_nanos),
                  bounds.left, bounds.top, bounds.right, bounds.bottom)) return false;
        if (!open_native()) return false;
        mpw_token fresh{};
        if (!absent_setup(geometry_, fresh, seen_) ||
            !expect(seen_.stream == session_stream_, "fresh_session_stream_mismatch") ||
            !verify_retained()) return false;
        // Finally prove the independently retained frame without any result,
        // then the mapping without any frame. Never dereference discarded views.
        retained_.info = {};
        retained_.match = {};
        retained_.result.reset();
        madopilot::Operation op;
        madopilot::Mapping remapped;
        madopilot::Image image;
        if (!operation(op) || !map_frame(retained_.frame, retained_.geometry, op, remapped, image) ||
            !expect(token_matches(image, retained_.geometry, retained_.token), "frame_after_result_mismatch")) return false;
        remapped.reset();
        retained_.frame.reset();
        // A second fresh command proves capture also progresses with only the
        // old mapping retained; frame/result ownership cannot hide pinning.
        if (!visual(false, fresh) || !observe(geometry_, fresh, seen_)) return false;
        const auto surviving = retained_.mapping.describe();
        const auto stamp = retained_.mapping.stamp();
        return accept(surviving, "mapping_after_frame_failed") && accept(stamp, "mapping_after_frame_stamp_failed") &&
               expect(same_stamp(stamp.value(), retained_source) &&
                          token_matches(surviving.value(), retained_.geometry, retained_.token),
                      "mapping_after_frame_mismatch") &&
               fact("mapped_view_bytes", "%llu", number(surviving.value().bytes.size()));
    }

    bool lifecycle_pending(madopilot::TemplateQuerySnapshot& snapshot, bool new_session)
    {
        if (new_session && !open_native()) return false;
        mpw_token absent{};
        if (new_session) {
            if (!absent_setup(geometry_, absent, seen_)) return false;
        } else if (!visual(false, absent) || !observe(geometry_, absent, seen_)) return false;
        return start_pending(geometry_, seen_, query_, snapshot);
    }

    bool terminal(madopilot::TemplateQueryOutcome outcome, madopilot::Status status,
                  std::uint64_t id)
    {
        madopilot::TemplateQueryResult result;
        if (!wait(query_, result) || !nonmatched(result, outcome, status, id)) return false;
        auto repeat = query_.cancel();
        if (!accept(repeat, "lifecycle_repeat_cancel_failed") || !nonmatched(repeat.value(), outcome, status, id)) return false;
        query_.reset();
        return true;
    }

    bool lifecycle_terminals()
    {
        madopilot::TemplateQuerySnapshot snapshot{};
        if (!lifecycle_pending(snapshot, false) || !close_session() ||
            !terminal(MADOPILOT_TEMPLATE_QUERY_OUTCOME_SESSION_CLOSED, MADOPILOT_STATUS_CLOSED,
                      snapshot.query_id)) return false;
        session_.reset();
        engine_.reset();
        if (!lifecycle_pending(snapshot, true)) return false;
        engine_.reset(); // Targets, templates, packages, probes and clones are already gone.
        if (!terminal(MADOPILOT_TEMPLATE_QUERY_OUTCOME_SCHEDULER_CLOSED, MADOPILOT_STATUS_CLOSED,
                      snapshot.query_id) || !close_session()) return false;
        session_.reset();
        if (!lifecycle_pending(snapshot, true)) return false;
        if (mpw_action("DESTROY") != 1) return fail("target_destroy_failed");
        if (!terminal(MADOPILOT_TEMPLATE_QUERY_OUTCOME_TARGET_LOST, MADOPILOT_STATUS_TARGET_LOST,
                      snapshot.query_id)) return false;
        return close_session();
    }

    int finish()
    {
        current_ = 8;
        reason_ = nullptr;
        status_ = MADOPILOT_STATUS_OK;
        outcome_ = "PASS";
        bool clean = false;
        try {
            query_.reset();
            retained_.reset();
            clean = close_session();
        } catch (...) {
            clean = false;
        }
        session_.reset();
        engine_.reset();
        clean = clean && query_.empty() && retained_.result.empty() && retained_.frame.empty() &&
                retained_.mapping.empty() && session_.empty() && engine_.empty();
        for (std::size_t index = 0; index < 8 && protocol_ok_; ++index) {
            if (!reported_[index]) report(index, "UNEXECUTED", "prior_row_unavailable");
        }
        const bool measured = protocol_ok_ && resource_sample(2, 4);
        if (protocol_ok_) fact("status", "%d %d",
                               static_cast<int>(status_),
                               static_cast<int>(MADOPILOT_TEMPLATE_QUERY_OUTCOME_NONE));
        if (protocol_ok_) {
            if (!measured) report(8, "INFRA", "resource_snapshot_failed");
            else report(8, clean ? "PASS" : "FAIL",
                        clean ? "consumer_cleanup" : "consumer_cleanup_failed");
        }
        // These OS samples are not GPU-byte or exact native-object counts.
        // Consumer PASS means cleanup and measurement, not resource-budget
        // acceptance. The controller withholds F9 until independent native
        // resource ceilings and fixture/resource finalization both qualify.
        if (protocol_ok_ && mpw_action("DONE") != 1) protocol_ok_ = false;
        // Row outcomes carry semantic failures; process status carries transport failure.
        return protocol_ok_ ? 0 : 1;
    }

    madopilot::Api api_;
    std::string_view title_;
    madopilot::Engine engine_;
    madopilot::Session session_;
    madopilot::TemplateQuery query_;
    Retained retained_;
    Geometry geometry_;
    madopilot::FrameStamp seen_{};
    std::uint64_t target_ = 0;
    std::uint64_t session_stream_ = 0;
    std::array<char, 128> backend_version_{};
    std::size_t backend_version_size_ = 0;
    std::uint64_t opened_at_ = 0;
    std::uint64_t observed_at_ = 0;
    std::array<bool, 9> reported_{};
    std::size_t current_ = 0;
    const char* reason_ = nullptr;
    const char* outcome_ = "PASS";
    madopilot::Status status_ = MADOPILOT_STATUS_OK;
    bool protocol_ok_ = true;
};

int check(const madopilot::Api& api)
{
    const auto build = api.describe_build();
    if (!build || build.value().abi_major != 1 || build.value().abi_minor < 6 ||
        build.value().table_size < MADOPILOT_API_SIZE_ABI_1_6 ||
        api.extent() < MADOPILOT_API_SIZE_TEMPLATE_QUERY_REQUIRED ||
        api.extent() < MADOPILOT_API_SIZE_TEMPLATE_QUERY_RESULT_REQUIRED) {
        std::puts("CHECK FAIL abi_extent");
        return 1;
    }
    std::printf("CHECK ABI %u %u %u\n", build.value().abi_major, build.value().abi_minor, build.value().table_size);
#if !defined(__APPLE__) && !defined(_WIN32)
    std::puts("CHECK UNSUPPORTED native_platform");
    return 0;
#else
    const auto now = api.clock_now();
    if (!now || now.value() > std::numeric_limits<std::uint64_t>::max() - operation_nanos) return 1;
    madopilot::Operation op;
    op.deadline(now.value() + operation_nanos);
    auto created = api.create_engine(native_source(), op);
    if (!created) {
        std::printf("CHECK REFUSED %d\n", static_cast<int>(created.status()));
        return created.status() == MADOPILOT_STATUS_UNSUPPORTED ? 0 : 1;
    }
    auto engine = created.take();
    const auto capabilities = engine.capabilities();
    const auto scheduler = engine.template_scheduler_descriptor();
    if (!capabilities || !scheduler || !fixed_scheduler(scheduler.value())) return 1;
    std::printf("CHECK CAPABILITIES %u\n", capabilities.value().flags);
    constexpr madopilot::PermissionKind kinds[] = {
        MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE, MADOPILOT_PERMISSION_KIND_INPUT_CONTROL
    };
    for (const auto kind : kinds) {
        const auto instant = api.clock_now();
        if (!instant || instant.value() > std::numeric_limits<std::uint64_t>::max() - operation_nanos) return 1;
        op.deadline(instant.value() + operation_nanos);
        const auto permission = engine.permission(kind, op);
        if (permission) {
            if (!capabilities.value().reads_permissions() || permission.value().kind != kind) return 1;
            std::printf("CHECK PERMISSION %d %d\n", static_cast<int>(kind), static_cast<int>(permission.value().state));
        } else {
            if (capabilities.value().reads_permissions() || permission.status() != MADOPILOT_STATUS_UNSUPPORTED) return 1;
            std::printf("CHECK PERMISSION_UNSUPPORTED %d %d\n", static_cast<int>(kind), static_cast<int>(permission.status()));
        }
    }
    engine.reset();
    std::puts("CHECK COMPLETE admission_only_no_native_pass");
    return 0;
#endif
}

} // namespace

int main(int argc, char** argv)
{
    if (std::setlocale(LC_ALL, "C") == nullptr) return 1;
    const bool check_only = argc == 2 && std::string_view(argv[1]) == "--check";
    const bool native = argc == 4 && std::string_view(argv[1]) == "--native" &&
                        std::string_view(argv[2]) == "--title" && argv[3][0] != '\0' &&
                        std::string_view(argv[3]).find_first_of("\r\n") == std::string_view::npos;
    if (!check_only && !native) {
        std::fputs("usage: native-template-watch --check | --native --title <exact fixture title>\n", stderr);
        return 2;
    }
    if (native) {
        // The actual loaded image is independently hashed by the controller.
        // This exported address is loader evidence, not a raw watcher call.
        if (!mpw_library(reinterpret_cast<const void*>(&madopilot_get_api))) return 1;
    }
    try {
        auto loaded = madopilot::Api::load();
        if (!loaded) {
            if (check_only) {
                std::printf("CHECK REFUSED %d\n", static_cast<int>(loaded.status()));
            } else {
                char status[48];
                std::snprintf(status, sizeof(status), "%d %d", static_cast<int>(loaded.status()),
                              static_cast<int>(MADOPILOT_TEMPLATE_QUERY_OUTCOME_NONE));
                const char* outcome = loaded.status() == MADOPILOT_STATUS_UNSUPPORTED ? "UNSUPPORTED" : "FAIL";
                if (!mpw_fact("F1", "status", status) ||
                    !mpw_row("F1", outcome, "abi_negotiation_refused")) return 1;
                for (std::size_t index = 1; index < 8; ++index) {
                    if (!mpw_row(rows[index], "UNEXECUTED", "abi_unavailable")) return 1;
                }
                mpw_resource_snapshot resources{};
                const bool measured = mpw_resources(&resources) == 1;
                if (measured) {
                    char values[160];
                    std::snprintf(values, sizeof(values), "2 4 %llu %llu %llu",
                                  number(resources.private_or_footprint_bytes), number(resources.resident_bytes),
                                  number(resources.handle_or_port_count));
                    if (!mpw_fact("F9", "resource", values)) return 1;
                }
                if (!mpw_fact("F9", "status", "0 0") ||
                    !mpw_row("F9", measured ? "PASS" : "INFRA",
                             measured ? "consumer_cleanup" : "resource_snapshot_failed") ||
                    mpw_action("DONE") != 1) return 1;
                return 0;
            }
            return loaded.status() == MADOPILOT_STATUS_UNSUPPORTED ? 0 : 1;
        }
        const auto api = loaded.take();
        if (check_only) return check(api);
        Qualification qualification(api, argv[3]);
        return qualification.run();
    } catch (...) {
        std::fputs("consumer initialization failed\n", stderr);
        return 1;
    }
}
