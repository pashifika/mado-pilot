/* Integrated default OCR through the thin C++ ABI 1.3 wrapper. */

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
        std::fprintf(stderr, "default OCR prerequisites are not configured; skipping\n");
        return 77;
    }
    const std::string controlled_model_root = controlled_path(model_root);
    const std::string controlled_runtime_path = controlled_path(runtime_path);

    auto loaded = madopilot::Api::load();
    madopilot::Api api;
    if (!take(loaded, api, "Api::load") ||
        api.extent() < MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_DEFAULT_OCR) {
        return 1;
    }
    auto described = api.describe_build();
    madopilot::BuildInfo build;
    if (!take(described, build, "describe_build")) return 1;

    std::vector<std::uint8_t> pixels(64u * 64u * 4u, 0);
    madopilot::ReplayFrame supplied;
    supplied.extent(64, 64)
        .format(MADOPILOT_PIXEL_FORMAT_BGRA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(pixels.data(), pixels.size());
    auto source = madopilot::Source::replay_memory("default-ocr-blank");
    source.frame(supplied);
    madopilot::EngineOptions options;
    madopilot::DefaultOcrOptions default_ocr(controlled_model_root,
                                              controlled_runtime_path);
    madopilot::Operation operation;
    auto built = api.create_engine_with_default_ocr(
        source, options, default_ocr, operation);
    madopilot::Engine engine;
    if (!take(built, engine, "create_engine_with_default_ocr")) return 1;

    auto capabilities = engine.capabilities();
    if (!capabilities || !capabilities.value().has_ocr()) {
        std::fprintf(stderr, "default engine does not report OCR\n");
        return 1;
    }
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

    madopilot::OcrRequest request;
    request.frame(frame)
        .model(build.default_ocr_model.view())
        .backend(build.default_ocr_backend.view(),
                 build.default_ocr_backend_version.view())
        .output_space(MADOPILOT_SPACE_CAPTURE_PIXELS)
        .full_frame();
    auto recognized_full = session.recognize(request, operation);
    madopilot::OcrResult full;
    if (!take(recognized_full, full, "recognize full")) return 1;
    const auto full_info = full.describe();
    if (!full_info || full_info.value().region_count != 0) return 1;

    request.region(madopilot::Rect{MADOPILOT_SPACE_CAPTURE_PIXELS, 8, 8, 40, 40})
        .clip_policy(MADOPILOT_CLIP_POLICY_REJECT);
    auto recognized_region = session.recognize(request, operation);
    madopilot::OcrResult region;
    if (!take(recognized_region, region, "recognize region")) return 1;
    const auto region_info = region.describe();
    if (!region_info || region_info.value().region_count != 0 ||
        region_info.value().source.sequence != full_info.value().source.sequence ||
        region_info.value().effective_region.left != 8 ||
        region_info.value().effective_region.top != 8 ||
        region_info.value().effective_region.right != 40 ||
        region_info.value().effective_region.bottom != 40) {
        return 1;
    }

    if (!session.close(operation) || !session.close(operation)) return 1;
    frame.reset();
    session.reset();
    targets.reset();
    engine.reset();

    std::printf("default-ocr: backend=%.*s model=%.*s full=%llu region=%llu\n",
                static_cast<int>(build.default_ocr_backend.size()),
                build.default_ocr_backend.data(),
                static_cast<int>(build.default_ocr_model.size()),
                build.default_ocr_model.data(),
                static_cast<unsigned long long>(full_info.value().region_count),
                static_cast<unsigned long long>(region_info.value().region_count));
    return 0;
}
