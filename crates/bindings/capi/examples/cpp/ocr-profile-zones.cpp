/* Explicit bounded OCR profile and grouped zones through C++ ABI 1.4. */

#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

#include <madopilot/madopilot.hpp>

namespace {

template <class T>
bool take(madopilot::Result<T>& result, T& out, const char* call)
{
    if (!result) {
        std::fprintf(stderr, "%s failed with status %d\n", call,
                     static_cast<int>(result.status()));
        return false;
    }
    out = result.take();
    return true;
}

std::string controlled_path(const char* value)
{
    std::string path = std::filesystem::canonical(value).string();
#ifdef _WIN32
    if (path.rfind(R"(\\?\)", 0) != 0) {
        path = R"(\\?\)" + path;
    }
#endif
    return path;
}

} // namespace

int main(int argc, char** argv)
{
    const char* model_root = nullptr;
    const char* runtime_path = nullptr;
    for (int index = 1; index + 1 < argc; index += 2) {
        if (std::string_view(argv[index]) == "--model-root") {
            model_root = argv[index + 1];
        } else if (std::string_view(argv[index]) == "--runtime") {
            runtime_path = argv[index + 1];
        }
    }
    if (model_root == nullptr) model_root = std::getenv("MADO_PILOT_G004_MODEL_ROOT");
    if (runtime_path == nullptr) runtime_path = std::getenv("MADO_PILOT_ONNX_RUNTIME");
    if (model_root == nullptr || runtime_path == nullptr) {
        std::fprintf(stderr, "profile OCR prerequisites are not configured; skipping\n");
        return 77;
    }
    const std::string controlled_model_root = controlled_path(model_root);
    const std::string controlled_runtime = controlled_path(runtime_path);

    auto loaded = madopilot::Api::load();
    madopilot::Api api;
    if (!take(loaded, api, "Api::load") ||
        api.extent() < MADOPILOT_API_SIZE_ENGINE_OCR_DESCRIPTOR) {
        return 1;
    }
    const auto build_result = api.describe_build();
    if (!build_result) return 1;
    const auto& build = build_result.value();

    std::vector<std::uint8_t> pixels(64u * 64u * 4u, 0);
    madopilot::ReplayFrame supplied;
    supplied.extent(64, 64)
        .format(MADOPILOT_PIXEL_FORMAT_BGRA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .stride(64u * 4u)
        .pixels(pixels.data(), pixels.size());
    auto source = madopilot::Source::replay_memory("bounded-profile-blank");
    source.frame(supplied);
    madopilot::EngineOptions options;
    madopilot::OcrProfileOptions profile(
        MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR, controlled_model_root,
        controlled_runtime);
    madopilot::Operation operation;

    auto built = api.create_engine_with_ocr_profile(
        source, options, profile, operation);
    madopilot::Engine engine;
    if (!take(built, engine, "create_engine_with_ocr_profile")) return 1;
    const auto descriptor_result = engine.ocr_descriptor();
    if (!descriptor_result ||
        descriptor_result.value().backend_id.view() !=
            build.default_ocr_backend.view() ||
        descriptor_result.value().backend_version.view() !=
            build.default_ocr_backend_version.view() ||
        descriptor_result.value().model_id.view() !=
            build.bounded_ocr_model.view() ||
        descriptor_result.value().model_version.view() !=
            build.bounded_ocr_model_version.view() ||
        descriptor_result.value().profile_id.view() !=
            build.bounded_ocr_profile.view()) {
        return 1;
    }
    const auto& descriptor = descriptor_result.value();
    auto discovered = engine.discover(operation);
    madopilot::TargetList targets;
    if (!take(discovered, targets, "discover")) return 1;
    madopilot::OpenRequest open;
    auto opened = engine.open_session(targets, 0, open, operation);
    madopilot::Session session;
    if (!take(opened, session, "open_session")) return 1;
    auto acquired = session.acquire_frame(operation);
    madopilot::Frame frame;
    if (!take(acquired, frame, "acquire_frame")) return 1;

    madopilot::OcrRequest singular_request;
    singular_request.frame(frame)
        .model(descriptor.model_id.view())
        .backend(descriptor.backend_id.view(), descriptor.backend_version.view())
        .output_space(MADOPILOT_SPACE_CAPTURE_PIXELS);
    auto recognized = session.recognize(singular_request, operation);
    madopilot::OcrResult singular_result;
    if (!take(recognized, singular_result, "recognize")) return 1;
    const auto singular_info = singular_result.describe();
    if (!singular_info ||
        singular_info.value().model_id.view() != descriptor.model_id.view() ||
        singular_info.value().profile_id.view() != descriptor.profile_id.view()) {
        return 1;
    }
    singular_result.reset();

    madopilot::ZoneScanOcrRequest request;
    request.frame(frame)
        .model(descriptor.model_id.view())
        .backend(descriptor.backend_id.view(), descriptor.backend_version.view())
        .output_space(MADOPILOT_SPACE_CAPTURE_PIXELS)
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 24, 24})
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 40, 0, 64, 24})
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 40, 24, 64});
    auto scanned = session.scan_ocr_zones(request, operation);
    madopilot::ZoneScanOcrResult result;
    if (!take(scanned, result, "scan_ocr_zones")) return 1;

    if (!session.close(operation)) return 1;
    frame.reset();
    session.reset();
    targets.reset();
    engine.reset();

    const auto info = result.describe();
    if (!info || info.value().zone_count != 3 ||
        info.value().unique_candidate_count != 0 ||
        info.value().membership_count != 0) {
        return 1;
    }
    for (std::size_t zone_index = 0;
         zone_index < static_cast<std::size_t>(info.value().zone_count);
         ++zone_index) {
        const auto group = result.zone_at(zone_index);
        if (!group) return 1;
        for (std::size_t region_index = 0;
             region_index < static_cast<std::size_t>(group.value().region_count);
             ++region_index) {
            const auto text = result.text_at(zone_index, region_index);
            if (!text) return 1;
            std::printf("%.*s\n", static_cast<int>(text.value().size()),
                        text.value().data());
        }
    }
    result.reset();
    return 0;
}
