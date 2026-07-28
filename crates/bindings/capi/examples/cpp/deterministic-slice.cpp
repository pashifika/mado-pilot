/*
 * The complete deterministic Phase 1 flow, in C++.
 *
 * The same questions and the same numbers as
 * crates/bindings/capi/examples/c/deterministic-slice.c, asked through the
 * header-only RAII wrapper instead of the raw table: negotiate, build an
 * absolute deadline and a cancellation token, supply a deterministic replay
 * source, discover its target, open a session, take a frame and map it, load a
 * tracked asset package, prepare two templates, search that exact frame for
 * both, read the source-correlated results, ask for a template the package does
 * not declare and read the structured error, and close.
 *
 * What differs from the C version is the ownership: there is no cleanup block
 * and no release call anywhere below. Every owner is move-only and releases in
 * its destructor, in reverse construction order, on every path including the
 * early returns. The one thing that stays explicit is `close`, because a
 * destructor cannot report a failed drain.
 *
 * Nothing here throws, and nothing here catches: the wrapper reports every
 * MadoPilot failure as a `Result`.
 *
 * The scene comes from ../deterministic-scene.h, the same header the C example
 * includes, which is the same integer arithmetic `mado-pilot-testkit` uses.
 *
 *   usage: deterministic-slice-cpp --package <dir> [--label <text>]
 */

#include <cstdio>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "deterministic-scene.h"
#include "madopilot/madopilot.hpp"

namespace {

int failures = 0;
madopilot::Api api;

void print_str(const char* label, std::string_view text)
{
    std::printf("%s%.*s", label, static_cast<int>(text.size()), text.data());
}

std::string status_name(madopilot::Status status)
{
    return api.status_text(status).to_string();
}

/* Reports a failed expectation and keeps going, so one run shows every problem
 * rather than only the first. */
bool expect(bool condition, const char* what)
{
    if (!condition) {
        std::printf("FAIL: %s\n", what);
        failures += 1;
    }
    return condition;
}

/* Prints a failure that came back in a `Result`. The message is owned by the
 * error, so there is nothing to copy before it goes out of scope. */
void report_error(const char* what, const madopilot::Error& error)
{
    std::printf("  %s: status %s category %d", what, status_name(error.status()).c_str(),
                static_cast<int>(error.category()));
    if (error.asset_detail().has_value()) {
        std::printf(" asset_fault %d stage %d",
                    static_cast<int>(error.asset_detail()->fault),
                    static_cast<int>(error.asset_detail()->stage));
    }
    if (error.backend().has_value()) {
        std::printf(" backend %s", error.backend()->c_str());
    }
    print_str("\n    ", error.message());
    std::printf("\n");
}

/* Checks a `Result` and reports the error it carries. */
template <class T>
bool expect_ok(const madopilot::Result<T>& result, const char* what)
{
    if (!result) {
        std::printf("FAIL: %s returned %s (%d)\n", what,
                    status_name(result.status()).c_str(),
                    static_cast<int>(result.status()));
        report_error(what, result.error());
        failures += 1;
        return false;
    }
    return true;
}

void usage(const char* program)
{
    std::fprintf(stderr, "usage: %s --package <dir> [--label <text>]\n", program);
}

} // namespace

int main(int argc, char** argv)
{
    const char* package_path = nullptr;
    const char* label = "unlabelled host";

    for (int index = 1; index < argc; ++index) {
        const std::string_view argument(argv[index]);
        if (argument == "--package" && index + 1 < argc) {
            package_path = argv[++index];
        } else if (argument == "--label" && index + 1 < argc) {
            label = argv[++index];
        } else {
            usage(argv[0]);
            return 2;
        }
    }
    if (package_path == nullptr) {
        usage(argv[0]);
        return 2;
    }

    // 1. Negotiate the table. Nothing else can be called until this succeeds.
    {
        auto loaded = madopilot::Api::load();
        if (!loaded) {
            std::fprintf(stderr, "madopilot::Api::load failed with %d\n",
                         static_cast<int>(loaded.status()));
            return 1;
        }
        api = loaded.take();
    }
    std::printf("host: %s\n", label);

    const auto build = api.describe_build();
    if (!expect_ok(build, "describe_build")) {
        return 1;
    }
    std::printf("abi: %u.%u table %u bytes (header declares %zu)\n",
                build.value().abi_major, build.value().abi_minor,
                build.value().table_size, sizeof(madopilot_api_t));
    print_str("library: ", build.value().library_version);
    print_str(" backend ", build.value().required_backend);
    std::printf("\n");

    // 2. Build an absolute deadline and a cancellation token. The deadline is an
    //    instant in the library's own monotonic domain, not a duration.
    const auto now = api.clock_now();
    if (!expect_ok(now, "clock_now")) {
        return 1;
    }
    auto cancellation = api.create_cancellation();
    if (!expect_ok(cancellation, "create_cancellation")) {
        return 1;
    }

    madopilot::Operation operation;
    operation.deadline(now.value() + 30ull * 1000ull * 1000ull * 1000ull)
        .cancellation(cancellation.value());

    // 3. Supply the deterministic scene as a memory replay source. The wrapper
    //    borrows these pixels only for the duration of `create_engine`, which
    //    copies them; the vector owns its storage throughout and releases it at
    //    the end of `main`.
    std::vector<std::uint8_t> scene(SCENE_BYTES);
    scene_fill_rgba(scene.data());

    madopilot::ReplayFrame supplied;
    supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(scene.data(), scene.size());

    auto source = madopilot::Source::replay_memory("panel");
    source.frame(supplied);

    auto engine_result = api.create_engine(source, operation);
    if (!expect_ok(engine_result, "create_engine")) {
        return 1;
    }
    const madopilot::Engine engine = engine_result.take();

    // 4. Discover, and open the one target.
    auto targets_result = engine.discover(operation);
    if (!expect_ok(targets_result, "discover")) {
        return 1;
    }
    madopilot::TargetList targets = targets_result.take();

    const auto count = targets.count();
    if (!expect_ok(count, "target count")) {
        return 1;
    }
    expect(count.value() == 1, "the replay source declares exactly one target");

    const auto target = targets.at(0);
    if (expect_ok(target, "target at 0")) {
        print_str("target: ", target.value().name);
        print_str(" from ", target.value().provider);
        std::printf(" %ux%u format %d\n", target.value().width, target.value().height,
                    static_cast<int>(target.value().format));
    }

    // Out of range is refused by the C boundary, and the wrapper hands the
    // refusal back rather than throwing.
    const auto beyond = targets.at(count.value());
    expect(!beyond && beyond.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "an out-of-range target index is invalid argument");

    madopilot::OpenRequest open_request;
    open_request.require_format(MADOPILOT_PIXEL_FORMAT_RGBA8);

    auto session_result = engine.open_session(targets, 0, open_request, operation);
    if (!expect_ok(session_result, "open_session")) {
        return 1;
    }
    const madopilot::Session session = session_result.take();

    // Opening copied the identity, so the list is no longer needed. Every
    // borrowed target string dies with it; nothing below uses one.
    targets.reset();

    const auto session_info = session.describe();
    if (expect_ok(session_info, "session describe")) {
        std::printf("session: stream %llu %ux%u\n",
                    static_cast<unsigned long long>(session_info.value().stream),
                    session_info.value().width, session_info.value().height);
    }

    // 5. Take one frame and hold it. Everything below searches this exact frame,
    //    not whatever the session publishes later.
    auto frame_result = session.frame(operation);
    if (!expect_ok(frame_result, "session frame")) {
        return 1;
    }
    const madopilot::Frame frame = frame_result.take();

    const auto stamp = frame.stamp();
    if (expect_ok(stamp, "frame stamp")) {
        std::printf("frame: stream %llu epoch %llu sequence %llu geometry %llu\n",
                    static_cast<unsigned long long>(stamp.value().stream),
                    static_cast<unsigned long long>(stamp.value().epoch),
                    static_cast<unsigned long long>(stamp.value().sequence),
                    static_cast<unsigned long long>(stamp.value().geometry));
        expect(stamp.value().epoch == 0 && stamp.value().sequence == 0 &&
                   stamp.value().geometry == 0,
               "a static image publishes epoch 0, sequence 0, geometry 0");
    }

    const auto frame_info = frame.describe();
    if (expect_ok(frame_info, "frame describe")) {
        std::printf("frame geometry: %ux%u stride %llu bounds [%d, %d) x [%d, %d)\n",
                    frame_info.value().width, frame_info.value().height,
                    static_cast<unsigned long long>(frame_info.value().stride),
                    frame_info.value().bounds.left, frame_info.value().bounds.right,
                    frame_info.value().bounds.top, frame_info.value().bounds.bottom);
    }

    // 6. Map it. The mapped bytes stay readable after the session is gone,
    //    because the mapping owns the storage its byte view borrows.
    madopilot::MapRequest map_request;
    map_request.format(MADOPILOT_PIXEL_FORMAT_RGBA8);

    auto mapping_result = frame.map(map_request, operation);
    if (!expect_ok(mapping_result, "frame map")) {
        return 1;
    }
    const madopilot::Mapping mapping = mapping_result.take();

    const auto image = mapping.describe();
    if (expect_ok(image, "mapping describe")) {
        std::printf("mapped: %ux%u %zu bytes shared %d\n", image.value().width,
                    image.value().height, image.value().bytes.size(),
                    image.value().shared() ? 1 : 0);
        expect(image.value().bytes.size() == SCENE_BYTES,
               "the whole frame maps to width * height * 4 bytes");
    }

    // 7. Load the tracked asset package and prepare its templates.
    auto package_result =
        engine.load_package(madopilot::PackageSource::directory(package_path), operation);
    if (!expect_ok(package_result, "load_package")) {
        return 1;
    }
    const madopilot::Package package = package_result.take();

    const auto package_info = package.describe();
    if (expect_ok(package_info, "package describe")) {
        print_str("package: ", package_info.value().package_id);
        print_str(" ", package_info.value().package_version);
        print_str(" under ", package_info.value().license);
        std::printf(", %llu templates\n",
                    static_cast<unsigned long long>(package_info.value().template_count));

        for (std::uint64_t at = 0; at < package_info.value().template_count; ++at) {
            const auto id = package.template_id(static_cast<std::size_t>(at));
            if (expect_ok(id, "package template id")) {
                print_str("  declares ", id.value());
                std::printf("\n");
            }
        }
    }

    auto present_result = engine.prepare_template(package, "panel.patch", operation);
    if (!expect_ok(present_result, "prepare_template(panel.patch)")) {
        return 1;
    }
    const madopilot::Template present = present_result.take();

    auto absent_result = engine.prepare_template(package, "panel.absent", operation);
    if (!expect_ok(absent_result, "prepare_template(panel.absent)")) {
        return 1;
    }
    const madopilot::Template absent = absent_result.take();

    const auto template_info = present.describe();
    if (expect_ok(template_info, "template describe")) {
        print_str("template: ", template_info.value().id);
        std::printf(" %ux%u min_score %.2f max_results %u\n", template_info.value().width,
                    template_info.value().height, template_info.value().min_score,
                    template_info.value().max_results);
    }

    // A package that loaded is valid. Asking it for an identity it never
    // declared is the caller's mistake, so it is invalid argument rather than an
    // asset failure — and the category still says the mistake is about the
    // package's contents.
    //
    // The `asset_fault`/`asset_stage` pair is not on this error: only package
    // loading carries it. `Error::asset_detail()` is empty here and populated by
    // the failing load below, which is the difference a C++ caller sees.
    {
        const auto refused =
            engine.prepare_template(package, "panel.absent.typo", operation);
        expect(!refused && refused.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
               "an undeclared template identity is invalid argument");
        expect(refused.error().category() == MADOPILOT_ERROR_CATEGORY_ASSET,
               "the refusal is categorized against the package's contents");
        std::printf("undeclared template:\n");
        report_error("prepare_template", refused.error());
    }

    // A package that cannot be read is the failure that does carry the pair:
    // which rule was broken, and how far loading had got. Collapsing them into
    // the status would lose detail the C ABI keeps on purpose.
    {
        const auto unreadable = engine.load_package(
            madopilot::PackageSource::directory("no-such-package-directory"), operation);
        expect(!unreadable, "an unreadable package source fails to load");
        if (expect(unreadable.error().asset_detail().has_value(),
                   "a failing load names the rule and the stage")) {
            std::printf("unreadable package: asset_fault %d stage %d\n",
                        static_cast<int>(unreadable.error().asset_detail()->fault),
                        static_cast<int>(unreadable.error().asset_detail()->stage));
        }
    }

    // 8. Search that exact frame. Two searches, two different answers.
    madopilot::FindRequest find;
    find.frame(frame).search_for(present);

    auto found_result = session.find(find, operation);
    if (!expect_ok(found_result, "find(panel.patch)")) {
        return 1;
    }
    const madopilot::MatchResult found = found_result.take();

    const auto info = found.describe();
    if (!expect_ok(info, "result describe")) {
        return 1;
    }
    print_str("found by ", info.value().backend_id);
    print_str(" ", info.value().backend_version);
    std::printf(": %llu match(es) in [%d, %d) x [%d, %d)\n",
                static_cast<unsigned long long>(info.value().match_count),
                info.value().searched.left, info.value().searched.right,
                info.value().searched.top, info.value().searched.bottom);
    expect(info.value().match_count == 2,
           "the patch is planted at two offsets in the scene");

    const auto effective = found.options();
    if (expect_ok(effective, "result options")) {
        std::printf("  ran with min_score %.2f max_results %u suppression %d\n",
                    effective.value().min_score, effective.value().max_results,
                    static_cast<int>(effective.value().suppression));
    }

    // Two byte-identical planted copies have no meaningful order: their scores
    // differ by far less than the comparison tolerance, so which one sorts first
    // is a property of the host's OpenCV build. Compare them as a set.
    const auto matches = found.matches();
    if (expect_ok(matches, "result matches")) {
        bool seen[2] = {false, false};
        for (const madopilot::Match& match : matches.value()) {
            print_str("  ", match.template_id);
            std::printf(" at [%d, %d) x [%d, %d) score %.6f\n", match.bounds.left,
                        match.bounds.right, match.bounds.top, match.bounds.bottom,
                        match.score);
            expect(match.bounds.space == MADOPILOT_SPACE_CAPTURE_PIXELS,
                   "a match rectangle names the space it is measured in");
            for (std::size_t planted = 0; planted < 2; ++planted) {
                if (match.bounds.left == static_cast<std::int32_t>(SCENE_PLANTED[planted][0]) &&
                    match.bounds.top == static_cast<std::int32_t>(SCENE_PLANTED[planted][1])) {
                    expect(!seen[planted], "each planted copy is reported once");
                    seen[planted] = true;
                }
            }
        }
        expect(seen[0] && seen[1],
               "both planted copies are found, in whichever order this host produced");
    }

    // An out-of-range index is the C boundary's refusal, carried through.
    const auto beyond_match =
        found.match_at(static_cast<std::size_t>(info.value().match_count));
    expect(!beyond_match && beyond_match.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "an out-of-range match index is invalid argument");

    // Nothing found is a successful answer to a well-formed question, so the
    // optional is empty rather than the result being a failure.
    madopilot::FindRequest find_absent;
    find_absent.frame(frame).search_for(absent);

    auto missing_result = session.find(find_absent, operation);
    if (!expect_ok(missing_result, "find(panel.absent)")) {
        return 1;
    }
    const madopilot::MatchResult missing = missing_result.take();

    const auto best = missing.first_match();
    if (expect_ok(best, "first_match on an empty result")) {
        expect(!best.value().has_value(), "the absent template is not on this frame");
    }
    const auto missing_info = missing.describe();
    if (expect_ok(missing_info, "absent result describe")) {
        std::printf("absent template: %llu match(es), which is a successful answer\n",
                    static_cast<unsigned long long>(missing_info.value().match_count));
        expect(missing_info.value().match_count == 0,
               "an empty answer still reports a count");
    }

    // 9. Close. Explicitly, and twice, because close is idempotent and because a
    //    destructor could report neither outcome.
    expect_ok(session.close(operation), "close");
    expect_ok(session.close(operation), "close again");
    const auto closed = session.is_closed();
    if (expect_ok(closed, "is_closed")) {
        expect(closed.value(), "the session reports itself closed");
    }

    const auto after = session.frame(operation);
    expect(!after && after.status() == MADOPILOT_STATUS_CLOSED,
           "a closed session publishes nothing further");

    // 10. What the caller owns survives the close, and survives the producer.
    const auto after_close = mapping.describe();
    if (expect_ok(after_close, "mapping describe after close")) {
        std::printf("mapping still readable after close: %zu bytes\n",
                    after_close.value().bytes.size());
    }
    const auto result_stamp = found.stamp();
    if (expect_ok(result_stamp, "result stamp after close")) {
        std::printf("result still correlated after close: sequence %llu\n",
                    static_cast<unsigned long long>(result_stamp.value().sequence));
    }

    if (failures != 0) {
        std::printf("%d expectation(s) failed\n", failures);
        return 1;
    }
    std::printf("deterministic slice complete\n");

    // Every owner above releases here, in reverse construction order. There is
    // no cleanup block, and no release call anywhere in this file.
    return 0;
}
