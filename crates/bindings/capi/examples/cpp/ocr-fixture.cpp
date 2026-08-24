/* Deterministic one-shot OCR through the thin C++ ABI 1.3 wrapper. */

#include "ocr-private-fixture.hpp"

#include <cstdio>
#include <string_view>
#include <vector>

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

} // namespace

int main(int argc, char** argv)
{
    const char* package_path = nullptr;
    for (int index = 1; index + 1 < argc; index += 2) {
        if (std::string_view(argv[index]) == "--package") {
            package_path = argv[index + 1];
        }
    }
    if (package_path == nullptr) {
        std::fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
        return 2;
    }

    auto loaded = madopilot::Api::load();
    madopilot::Api api;
    if (!take(loaded, api, "Api::load") ||
        api.extent() < MADOPILOT_API_SIZE_OCR_RESULT_TEXT_AT) {
        return 1;
    }

    std::vector<std::uint8_t> pixels(32u * 24u * 4u, 0x40);
    madopilot::ReplayFrame supplied;
    supplied.extent(32, 24)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(pixels.data(), pixels.size());
    auto source = madopilot::Source::replay_memory("ocr-panel");
    source.frame(supplied);
    madopilot::Operation operation;
    madopilot::EngineOptions options;
    options.diagnostics(MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG, 16);
    auto built = madopilot::private_fixture::OcrEngine::create(
        api, source, options, operation);
    madopilot::private_fixture::OcrEngine engine;
    if (!take(built, engine, "create_private_ocr_fixture_engine")) return 1;
    auto reader_result = engine.take_diagnostic_reader();
    if (!reader_result || !reader_result.value().has_value()) {
        std::fprintf(stderr, "diagnostic reader unavailable\n");
        return 1;
    }
    auto maybe_reader = reader_result.take();
    madopilot::DiagnosticReader reader = std::move(*maybe_reader);
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
    auto loaded_package = engine.load_package(
        madopilot::PackageSource::directory(package_path), operation);
    madopilot::Package package;
    if (!take(loaded_package, package, "load_package")) return 1;

    madopilot::OcrRequest request;
    request.frame(frame)
        .package(package)
        .model(MADOPILOT_FIXTURE_OCR_MODEL_ID)
        .backend(MADOPILOT_FIXTURE_OCR_BACKEND_ID,
                 MADOPILOT_FIXTURE_OCR_BACKEND_VERSION)
        .output_space(MADOPILOT_SPACE_CAPTURE_PIXELS)
        .full_frame();
    auto recognized = session.recognize(request, operation);
    madopilot::OcrResult result;
    if (!take(recognized, result, "recognize")) return 1;
    madopilot::OcrResult retained = result.clone();
    result.reset();

    const auto closed = session.close(operation);
    if (!closed) return 1;
    frame.reset();
    package.reset();
    session.reset();
    targets.reset();
    engine.reset();

    const auto info = retained.describe();
    const auto region = retained.region_at(0);
    const auto text = retained.text_at(0);
    if (!info || !region || !text || info.value().region_count != 1) {
        std::fprintf(stderr, "OCR result access failed\n");
        return 1;
    }

    auto drained = reader.drain();
    if (!drained) {
        std::fprintf(stderr, "diagnostic drain failed\n");
        return 1;
    }
    auto drain = drained.take();
    if (!drain.batch.has_value()) {
        std::fprintf(stderr, "diagnostic batch unavailable\n");
        return 1;
    }
    madopilot::DiagnosticBatch batch = std::move(*drain.batch);
    const auto batch_info = batch.describe();
    if (!batch_info) return 1;
    bool saw_admission = false;
    bool saw_terminal = false;
    for (std::uint64_t index = 0; index < batch_info.value().record_count; ++index) {
        const auto record_result = batch.record_at(static_cast<std::size_t>(index));
        if (!record_result) return 1;
        const auto& record = record_result.value();
        saw_admission =
            saw_admission ||
            (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED &&
             record.operation == MADOPILOT_DIAGNOSTIC_OPERATION_OCR_RECOGNITION);
        if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OCR) {
            saw_terminal =
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_MODEL_INSTANCE) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_TIMING) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESOURCES) &&
                !record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_PROFILE) &&
                record.ocr_profile ==
                    MADOPILOT_OCR_DIAGNOSTIC_PROFILE_UNSPECIFIED &&
                record.ocr_outcome ==
                    MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_RECOGNIZED &&
                record.ocr_model_instance != 0 && record.result_count == 1 &&
                record.frame.sequence == info.value().source.sequence;
        }
    }
    if (!saw_admission || !saw_terminal) {
        std::fprintf(stderr, "OCR diagnostics were incomplete\n");
        return 1;
    }
    std::printf("ocr: sequence=%llu text=%.*s confidence=%.5f\n",
                static_cast<unsigned long long>(info.value().source.sequence),
                static_cast<int>(text.value().size()), text.value().data(),
                region.value().confidence);
    return 0;
}
