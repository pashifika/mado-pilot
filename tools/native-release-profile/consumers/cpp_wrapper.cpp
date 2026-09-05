/* Public C++ wrapper consumer; the only product symbol is madopilot_get_api.
 * Matches c_abi.c's observable oracle. Owners use RAII; close errors stay explicit.
 */

#include <cstdint>
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <string>
#include <string_view>
#include <system_error>
#include <vector>

#include "deterministic-scene.h"
#include "madopilot/madopilot.hpp"

namespace {

/* The accepted blank OCR fixture: a zeroed 64x64 BGRA frame. */
constexpr std::uint32_t blank_extent = 64;
constexpr std::size_t blank_bytes =
    static_cast<std::size_t>(blank_extent) * static_cast<std::size_t>(blank_extent) * 4u;

madopilot::Api api;
const char* failed_check = nullptr;
madopilot::Status failed_status = MADOPILOT_STATUS_OK;

/* Records the first failed check; later failures never replace it. */
bool fail(const char* check, madopilot::Status status)
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

template <class T>
bool require_ok(const madopilot::Result<T>& result, const char* check)
{
    return result.ok() || fail(check, result.status());
}

/* A refused call: `expected` came back, and the typed error carries it too. */
template <class T>
bool refused(const madopilot::Result<T>& result, madopilot::Status expected, const char* check)
{
    return require(!result.ok() && result.status() == expected &&
                       result.error().status() == expected,
                   check);
}

bool same_stamp(const madopilot::FrameStamp& left, const madopilot::FrameStamp& right)
{
    return left.stream == right.stream && left.epoch == right.epoch &&
           left.sequence == right.sequence && left.geometry == right.geometry;
}

bool same_rect(const madopilot::Rect& rect, std::int32_t left, std::int32_t top,
               std::int32_t right, std::int32_t bottom)
{
    return rect.space == MADOPILOT_SPACE_CAPTURE_PIXELS && rect.left == left && rect.top == top &&
           rect.right == right && rect.bottom == bottom;
}

/* A static replay source's first publication: a nonzero stream at epoch 0,
 * sequence 0, geometry 0. */
bool first_publication(const madopilot::FrameStamp& stamp)
{
    return stamp.stream != 0 && stamp.epoch == 0 && stamp.sequence == 0 && stamp.geometry == 0;
}

/* Resolves a controlled prerequisite to the canonical absolute form the library
 * compares, under the \\?\ prefix on Windows as the C++ example passes it.
 * Empty when the path cannot be resolved. */
std::string controlled_path(const char* value)
{
    std::error_code error;
    std::string path = std::filesystem::canonical(std::filesystem::u8path(value), error).u8string();
    if (error) {
        return std::string();
    }
#ifdef _WIN32
    if (path.rfind(R"(\\?\)", 0) != 0) {
        path = path.rfind(R"(\\)", 0) == 0 ? R"(\\?\UNC\)" + path.substr(2) : R"(\\?\)" + path;
    }
#endif
    return path;
}

/* Both planted copies, each reported once at its planted origin with the patch
 * extent, over the whole scene, under the searched frame's identity. */
bool planted_found(const madopilot::MatchResult& result, const madopilot::FrameStamp& stamp)
{
    const auto info = result.describe();
    const auto searched = result.stamp();
    const auto matches = result.matches();
    if (!info || !searched || !matches || info.value().match_count != 2 ||
        !same_stamp(searched.value(), stamp) ||
        !same_rect(info.value().searched, 0, 0, static_cast<std::int32_t>(SCENE_WIDTH),
                   static_cast<std::int32_t>(SCENE_HEIGHT))) {
        return false;
    }
    bool seen[2] = {false, false};
    for (const madopilot::Match& match : matches.value()) {
        if (match.template_id.view() != "panel.patch" ||
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

/* A successful search that qualified nothing, under the searched frame's identity. */
bool nothing_found(const madopilot::MatchResult& result, const madopilot::FrameStamp& stamp)
{
    const auto info = result.describe();
    const auto searched = result.stamp();
    return info && searched && info.value().match_count == 0 &&
           same_stamp(searched.value(), stamp) &&
           same_rect(info.value().searched, 0, 0,
                     static_cast<std::int32_t>(SCENE_WIDTH), static_cast<std::int32_t>(SCENE_HEIGHT));
}

/* The whole scene mapped back byte for byte, under the frame's identity. */
bool mapping_readable(const madopilot::Mapping& mapping, const madopilot::FrameStamp& stamp,
                      const std::vector<std::uint8_t>& scene)
{
    const auto image = mapping.describe();
    const auto mapped = mapping.stamp();
    return image && mapped && image.value().width == SCENE_WIDTH &&
           image.value().height == SCENE_HEIGHT && image.value().bytes.data() != nullptr &&
           image.value().bytes.size() == scene.size() &&
           std::memcmp(image.value().bytes.data(), scene.data(), scene.size()) == 0 &&
           same_stamp(mapped.value(), stamp);
}

/* An empty recognition of exactly the requested region by the build's default
 * OCR identity, under the recognized frame's identity. */
bool empty_recognition(const madopilot::OcrResult& result, const madopilot::FrameStamp& stamp,
                       std::int32_t left, std::int32_t top,
                       std::int32_t right, std::int32_t bottom)
{
    const auto info = result.describe();
    return info && info.value().region_count == 0 && same_stamp(info.value().source, stamp) &&
           same_rect(info.value().effective_region, left, top, right, bottom) &&
           info.value().output_space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
           info.value().backend_id.view() == "onnxruntime-cpu" &&
           info.value().backend_version.view() == "0.4.0+ort-1.29.0-api17" &&
           info.value().model_id.view() == "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1" &&
           info.value().model_version.view() == "rapidocr-3.9.2+095232a4c94f7f0e6600ba5bba1177010ad696d4" &&
           info.value().profile_id.view() == "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1";
}

/* One frame of packed pixels as a memory replay source named `name`. The pixels
 * are borrowed only for engine construction. */
madopilot::Source replay_source(const char* name, std::uint32_t width, std::uint32_t height,
                                madopilot::PixelFormat format,
                                const std::vector<std::uint8_t>& pixels)
{
    madopilot::ReplayFrame supplied;
    supplied.extent(width, height)
        .format(format)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(pixels.data(), pixels.size());
    auto source = madopilot::Source::replay_memory(name);
    source.frame(supplied);
    return source;
}

/* Discovers the one replay target and opens it. */
bool open_only_target(const madopilot::Engine& engine, const madopilot::Operation& operation,
                      const madopilot::OpenRequest& request, madopilot::Session& session,
                      const char* discover_check, const char* open_check)
{
    auto targets = engine.discover(operation);
    if (!require_ok(targets, discover_check)) {
        return false;
    }
    const auto count = targets.value().count();
    if (!require_ok(count, discover_check) || !require(count.value() == 1, discover_check)) {
        return false;
    }
    auto opened = engine.open_session(targets.value(), 0, request, operation);
    if (!require_ok(opened, open_check)) {
        return false;
    }
    session = opened.take();
    return true;
}

/* A token cancelled before any call carries it. */
bool cancelled_token(madopilot::Cancellation& token, const char* check)
{
    auto created = api.create_cancellation();
    if (!require_ok(created, check)) {
        return false;
    }
    token = created.take();
    const auto cancelled = token.cancel();
    const auto flagged = token.is_cancelled();
    return require_ok(cancelled, check) && require_ok(flagged, check) &&
           require(flagged.value(), check);
}

/* An operation whose deadline had already passed when it was read. */
bool expired_operation(madopilot::Operation& operation, const char* check)
{
    const auto now = api.clock_now();
    if (!require_ok(now, check)) {
        return false;
    }
    operation.deadline(now.value());
    return true;
}

/* Closes twice, confirms the closed state, and confirms a closed session
 * publishes nothing further. */
bool closed_idempotently(const madopilot::Session& session, const madopilot::Operation& operation,
                         const char* check)
{
    if (!require_ok(session.close(operation), check) ||
        !require_ok(session.close(operation), check)) {
        return false;
    }
    const auto closed = session.is_closed();
    if (!require_ok(closed, check) || !require(closed.value(), check)) {
        return false;
    }
    return refused(session.acquire_frame(operation), MADOPILOT_STATUS_CLOSED, check);
}

/* The deterministic matching workflow, with its refusal, close, and retention checks. */
bool matching_flow(const char* package_path, bool mapping_only)
{
    const madopilot::Operation operation;
    std::vector<std::uint8_t> scene(SCENE_BYTES);
    scene_fill_rgba(scene.data());
    auto pixels = scene;

    // 1. An engine over the deterministic scene, and its one open target.
    auto built = api.create_engine(
        replay_source("panel", SCENE_WIDTH, SCENE_HEIGHT, MADOPILOT_PIXEL_FORMAT_RGBA8, pixels),
        operation);
    auto poison = static_cast<volatile std::uint8_t*>(pixels.data());
    std::fill(poison, poison + pixels.size(), std::uint8_t{0xa5});
    std::vector<std::uint8_t>().swap(pixels);
    if (!require_ok(built, "matching-engine")) {
        return false;
    }
    madopilot::Engine engine = built.take();
    madopilot::OpenRequest open;
    open.require_format(MADOPILOT_PIXEL_FORMAT_RGBA8);
    madopilot::Session session;
    if (!open_only_target(engine, operation, open, session, "matching-discover", "matching-open")) {
        return false;
    }

    // 2. One frame, its complete identity, and a mapping that carries it.
    auto acquired = session.acquire_frame(operation);
    if (!require_ok(acquired, "matching-frame")) {
        return false;
    }
    madopilot::Frame frame = acquired.take();
    const auto stamped = frame.stamp();
    const auto described = frame.describe();
    if (!require_ok(stamped, "matching-frame") || !require_ok(described, "matching-frame") ||
        !require(first_publication(stamped.value()) && described.value().width == SCENE_WIDTH &&
                     described.value().height == SCENE_HEIGHT &&
                     same_rect(described.value().bounds, 0, 0,
                               static_cast<std::int32_t>(SCENE_WIDTH),
                               static_cast<std::int32_t>(SCENE_HEIGHT)),
                 "matching-frame")) {
        return false;
    }
    const madopilot::FrameStamp stamp = stamped.value();
    madopilot::MapRequest map_request;
    map_request.format(MADOPILOT_PIXEL_FORMAT_RGBA8);
    auto mapped = frame.map(map_request, operation);
    if (!require_ok(mapped, "matching-map")) {
        return false;
    }
    madopilot::Mapping mapping = mapped.take();
    if (!require(mapping_readable(mapping, stamp, scene), "matching-map")) {
        return false;
    }
    if (mapping_only) {
        if (!closed_idempotently(session, operation, "mapping-close")) return false;
        frame.reset();
        session.reset();
        engine.reset();
        if (!require(mapping_readable(mapping, stamp, scene), "mapping-retained")) return false;
        std::puts("MADO_PROFILE_MAPPING=retained");
        return true;
    }
    mapping.reset();

    // 3. The tracked package and its two templates.
    auto loaded =
        engine.load_package(madopilot::PackageSource::directory(package_path), operation);
    if (!require_ok(loaded, "package")) {
        return false;
    }
    madopilot::Package package = loaded.take();
    const auto package_info = package.describe();
    if (!require_ok(package_info, "package") ||
        !require(package_info.value().template_count == 2, "package")) {
        return false;
    }
    auto present_result = engine.prepare_from_package(package, "panel.patch", operation);
    if (!require_ok(present_result, "template-present")) {
        return false;
    }
    madopilot::Template present = present_result.take();
    auto absent_result = engine.prepare_from_package(package, "panel.absent", operation);
    if (!require_ok(absent_result, "template-absent")) {
        return false;
    }
    madopilot::Template absent = absent_result.take();

    // 4. Search that exact frame: two planted copies, nothing for the absent patch.
    madopilot::FindRequest find_present;
    find_present.frame(frame).search_for(present);
    auto found_result = session.find(find_present, operation);
    if (!require_ok(found_result, "find-present")) {
        return false;
    }
    madopilot::MatchResult found = found_result.take();
    if (!require(planted_found(found, stamp), "find-present")) {
        return false;
    }
    madopilot::FindRequest find_absent;
    find_absent.frame(frame).search_for(absent);
    auto missing_result = session.find(find_absent, operation);
    if (!require_ok(missing_result, "find-absent")) {
        return false;
    }
    madopilot::MatchResult missing = missing_result.take();
    if (!require(nothing_found(missing, stamp), "find-absent")) {
        return false;
    }
    std::puts("MADO_PROFILE_MATCHING=passed");

    // 5. Refusals: an already-cancelled token, then an already-passed deadline.
    madopilot::Cancellation token;
    if (!cancelled_token(token, "cancellation")) {
        return false;
    }
    madopilot::Operation cancelled;
    cancelled.cancellation(token);
    if (!refused(session.find(find_present, cancelled), MADOPILOT_STATUS_CANCELLED,
                 "cancellation")) {
        return false;
    }
    std::puts("MADO_PROFILE_CANCELLATION=refused");
    madopilot::Operation expired;
    if (!expired_operation(expired, "deadline") ||
        !refused(session.find(find_present, expired), MADOPILOT_STATUS_DEADLINE_EXCEEDED,
                 "deadline")) {
        return false;
    }
    std::puts("MADO_PROFILE_DEADLINE=refused");
    expired.cancellation(token);
    if (!refused(session.find(find_present, expired), MADOPILOT_STATUS_CANCELLED,
                 "cancellation-precedence")) return false;

    // 6. Close twice; a closed session publishes nothing further.
    if (!closed_idempotently(session, operation, "close")) {
        return false;
    }
    std::puts("MADO_PROFILE_CLOSE=idempotent");

    // 7. Reset every producer; what the caller owns stays readable and identical.
    present.reset();
    absent.reset();
    package.reset();
    frame.reset();
    session.reset();
    engine.reset();
    if (!require(planted_found(found, stamp) && nothing_found(missing, stamp),
                 "retained")) {
        return false;
    }
    std::puts("MADO_PROFILE_RETAINED=readable");
    return true;
}

/* The accepted CPU blank-frame OCR workflow, with the same refusal, close, and
 * retention checks. */
bool ocr_flow(const madopilot::BuildInfo& build, const char* model_root, const char* runtime_path)
{
    const madopilot::Operation operation;
    const std::string controlled_root = controlled_path(model_root);
    const std::string controlled_runtime = controlled_path(runtime_path);
    if (!require(!controlled_root.empty() && !controlled_runtime.empty(), "ocr-prerequisite")) {
        return false;
    }

    // 1. An engine with the accepted default CPU OCR over one blank frame.
    std::vector<std::uint8_t> blank(blank_bytes, 0);
    madopilot::EngineOptions options;
    madopilot::DefaultOcrOptions default_ocr(controlled_root, controlled_runtime);
    auto built = api.create_engine_with_default_ocr(
        replay_source("default-ocr-blank", blank_extent, blank_extent,
                      MADOPILOT_PIXEL_FORMAT_BGRA8, blank),
        options, default_ocr, operation);
    if (!require_ok(built, "ocr-engine")) {
        return false;
    }
    madopilot::Engine engine = built.take();
    auto blank_poison = static_cast<volatile std::uint8_t*>(blank.data());
    std::fill(blank_poison, blank_poison + blank.size(), std::uint8_t{0xa5});
    std::vector<std::uint8_t>().swap(blank);
    const auto provider = engine.ocr_provider_descriptor();
    if (!require_ok(provider, "ocr-provider") ||
        !require(provider.value().active_provider == MADOPILOT_OCR_EXECUTION_PROVIDER_CPU &&
                 provider.value().requested_policy == MADOPILOT_OCR_PROVIDER_POLICY_CPU &&
                 !provider.value().initialization_fell_back &&
                 provider.value().runtime_profile.view() == "onnxruntime-1.29.0-api17-cpu",
                 "ocr-provider")) {
        return false;
    }
    const auto capabilities = engine.capabilities();
    if (!require_ok(capabilities, "ocr-engine") ||
        !require(capabilities.value().has_ocr(), "ocr-engine")) {
        return false;
    }
    madopilot::Session session;
    if (!open_only_target(engine, operation, madopilot::OpenRequest(), session, "ocr-discover",
                          "ocr-open")) {
        return false;
    }

    // 2. One frame and its identity.
    auto acquired = session.acquire_frame(operation);
    if (!require_ok(acquired, "ocr-frame")) {
        return false;
    }
    madopilot::Frame frame = acquired.take();
    const auto stamped = frame.stamp();
    if (!require_ok(stamped, "ocr-frame") ||
        !require(first_publication(stamped.value()), "ocr-frame")) {
        return false;
    }
    const madopilot::FrameStamp stamp = stamped.value();
    const auto whole = static_cast<std::int32_t>(blank_extent);

    // 3. Recognize the whole frame and one region: a blank frame reads as nothing.
    madopilot::OcrRequest request;
    request.frame(frame)
        .model(build.default_ocr_model.view())
        .backend(build.default_ocr_backend.view(), build.default_ocr_backend_version.view())
        .output_space(MADOPILOT_SPACE_CAPTURE_PIXELS)
        .clip_policy(MADOPILOT_CLIP_POLICY_REJECT)
        .full_frame();
    auto full_result = session.recognize(request, operation);
    if (!require_ok(full_result, "ocr-full")) {
        return false;
    }
    madopilot::OcrResult full = full_result.take();
    if (!require(empty_recognition(full, stamp, 0, 0, whole, whole), "ocr-full")) {
        return false;
    }
    request.region(madopilot::Rect{MADOPILOT_SPACE_CAPTURE_PIXELS, 8, 8, 40, 40});
    auto bounded_result = session.recognize(request, operation);
    if (!require_ok(bounded_result, "ocr-region")) {
        return false;
    }
    madopilot::OcrResult bounded = bounded_result.take();
    if (!require(empty_recognition(bounded, stamp, 8, 8, 40, 40), "ocr-region")) {
        return false;
    }

    // 4. Refusals on recognition.
    madopilot::Cancellation token;
    if (!cancelled_token(token, "ocr-cancellation")) {
        return false;
    }
    madopilot::Operation cancelled;
    cancelled.cancellation(token);
    if (!refused(session.recognize(request, cancelled), MADOPILOT_STATUS_CANCELLED,
                 "ocr-cancellation")) {
        return false;
    }
    madopilot::Operation expired;
    if (!expired_operation(expired, "ocr-deadline") ||
        !refused(session.recognize(request, expired), MADOPILOT_STATUS_DEADLINE_EXCEEDED,
                 "ocr-deadline")) {
        return false;
    }
    expired.cancellation(token);
    if (!refused(session.recognize(request, expired), MADOPILOT_STATUS_CANCELLED,
                 "ocr-cancellation-precedence")) return false;

    // 5. Close twice; a closed session publishes nothing further.
    if (!closed_idempotently(session, operation, "ocr-close")) {
        return false;
    }

    // 6. Reset every producer; both results stay readable and identical.
    frame.reset();
    session.reset();
    engine.reset();
    if (!require(empty_recognition(full, stamp, 0, 0, whole, whole) &&
                     empty_recognition(bounded, stamp, 8, 8, 40, 40),
                 "ocr-retained")) {
        return false;
    }
    std::puts("MADO_PROFILE_OCR=passed");
    return true;
}

/* Prints the terminal line: 0 when every check held, 1 otherwise. */
int finish()
{
    if (failed_check == nullptr) {
        std::puts("MADO_PROFILE_RESULT=passed");
        return 0;
    }
    std::printf("MADO_PROFILE_FAILURE=%s\n", failed_check);
    if (failed_status != MADOPILOT_STATUS_OK) {
        const madopilot::BorrowedStr slug = api.status_text(failed_status);
        if (slug.empty()) {
            std::printf("MADO_PROFILE_STATUS=%d\n", static_cast<int>(failed_status));
        } else {
            std::printf("MADO_PROFILE_STATUS=%.*s\n", static_cast<int>(slug.size()),
                        slug.data());
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

    // Every observation reaches the log even if the library aborts afterwards.
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

    // Negotiate the table; nothing else can be called until this succeeds.
    auto loaded = madopilot::Api::load();
    if (!require_ok(loaded, "negotiate")) {
        return finish();
    }
    api = loaded.take();
    if (!require(api.extent() >= MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_DEFAULT_OCR, "negotiate")) {
        return finish();
    }
    const auto build = api.describe_build();
    if (!require_ok(build, "build")) {
        return finish();
    }
    std::puts("MADO_PROFILE_CONSUMER=cpp-wrapper");

    if (matching_flow(package, false) && matching_flow(package, true)) {
        ocr_flow(build.value(), model_root, runtime);
    }
    return finish();
}
