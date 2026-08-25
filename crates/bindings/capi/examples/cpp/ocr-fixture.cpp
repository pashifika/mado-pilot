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
        api.extent() < MADOPILOT_API_SIZE_OCR_ZONE_SCAN_RESULT_TEXT_AT) {
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
    options.diagnostics(MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG, 32);
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

    madopilot::ZoneScanOcrRequest one_zone_request;
    one_zone_request.frame(frame)
        .package(package)
        .model(MADOPILOT_FIXTURE_OCR_MODEL_ID)
        .backend(MADOPILOT_FIXTURE_OCR_BACKEND_ID,
                 MADOPILOT_FIXTURE_OCR_BACKEND_VERSION)
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 32, 24});
    auto one_scanned = session.scan_ocr_zones(one_zone_request, operation);
    madopilot::ZoneScanOcrResult one_result;
    if (!take(one_scanned, one_result, "scan one OCR zone")) return 1;
    const auto one_info = one_result.describe();
    const auto one_group = one_result.zone_at(0);
    if (!one_info || !one_group || one_info.value().zone_count != 1 ||
        one_info.value().unique_candidate_count != 1 ||
        one_info.value().membership_count != 1 ||
        one_group.value().region_count != 1) {
        return 1;
    }
    one_result.reset();

    madopilot::ZoneScanOcrRequest eight_zone_request;
    eight_zone_request.frame(frame)
        .package(package)
        .model(MADOPILOT_FIXTURE_OCR_MODEL_ID)
        .backend(MADOPILOT_FIXTURE_OCR_BACKEND_ID,
                 MADOPILOT_FIXTURE_OCR_BACKEND_VERSION);
    for (std::int32_t row = 0; row < 2; ++row) {
        for (std::int32_t column = 0; column < 4; ++column) {
            eight_zone_request.zone(
                {MADOPILOT_SPACE_CAPTURE_PIXELS, column * 8, row * 12,
                 column * 8 + 8, row * 12 + 12});
        }
    }
    auto eight_scanned =
        session.scan_ocr_zones(eight_zone_request, operation);
    madopilot::ZoneScanOcrResult eight_result;
    if (!take(eight_scanned, eight_result, "scan eight OCR zones")) return 1;
    const auto eight_info = eight_result.describe();
    const auto eight_empty = eight_result.zone_at(0);
    const auto eight_hit = eight_result.zone_at(1);
    if (!eight_info || !eight_empty || !eight_hit ||
        eight_info.value().zone_count != 8 ||
        eight_info.value().unique_candidate_count != 1 ||
        eight_info.value().membership_count != 1 ||
        !eight_empty.value().empty() || eight_hit.value().region_count != 1) {
        return 1;
    }
    eight_result.reset();

    madopilot::ZoneScanOcrRequest zone_request;
    zone_request.frame(frame)
        .package(package)
        .model(MADOPILOT_FIXTURE_OCR_MODEL_ID)
        .backend(MADOPILOT_FIXTURE_OCR_BACKEND_ID,
                 MADOPILOT_FIXTURE_OCR_BACKEND_VERSION)
        .output_space(MADOPILOT_SPACE_CAPTURE_PIXELS)
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 16, 12})
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 24, 0, 32, 12})
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 8, 0, 24, 12});
    auto scanned = session.scan_ocr_zones(zone_request, operation);
    madopilot::ZoneScanOcrResult grouped;
    if (!take(scanned, grouped, "scan_ocr_zones")) return 1;
    madopilot::ZoneScanOcrResult grouped_retained = grouped.clone();
    madopilot::ZoneScanOcrResult grouped_moved =
        std::move(grouped_retained);
    grouped.reset();

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

    const auto grouped_info = grouped_moved.describe();
    const auto grouped_first = grouped_moved.zone_at(0);
    const auto grouped_empty = grouped_moved.zone_at(1);
    const auto grouped_overlap = grouped_moved.zone_at(2);
    const auto grouped_region = grouped_moved.region_at(0, 0);
    const auto grouped_text = grouped_moved.text_at(0, 0);
    const auto overlap_text = grouped_moved.text_at(2, 0);
    if (!grouped_info || !grouped_first || !grouped_empty ||
        !grouped_overlap || !grouped_region || !grouped_text ||
        !overlap_text || grouped_info.value().zone_count != 3 ||
        grouped_info.value().unique_candidate_count != 1 ||
        grouped_info.value().membership_count != 2 ||
        grouped_info.value().source.sequence != info.value().source.sequence ||
        grouped_first.value().region_count != 1 ||
        !grouped_empty.value().empty() ||
        grouped_overlap.value().region_count != 1 ||
        grouped_region.value().confidence != region.value().confidence ||
        grouped_region.value().points[0].x != region.value().points[0].x ||
        grouped_region.value().points[0].y != region.value().points[0].y ||
        grouped_text.value().view() != text.value().view() ||
        grouped_text.value().view() != overlap_text.value().view()) {
        std::fprintf(stderr, "grouped OCR ownership/access failed\n");
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
    bool saw_singular_terminal = false;
    bool saw_grouped_terminal = false;
    for (std::uint64_t index = 0; index < batch_info.value().record_count; ++index) {
        const auto record_result = batch.record_at(static_cast<std::size_t>(index));
        if (!record_result) return 1;
        const auto& record = record_result.value();
        saw_admission =
            saw_admission ||
            (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED &&
             record.operation == MADOPILOT_DIAGNOSTIC_OPERATION_OCR_RECOGNITION);
        if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OCR &&
            record.result_count == 1 &&
            !record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_SOURCE_ENVELOPE)) {
            saw_singular_terminal =
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_MODEL_INSTANCE) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_TIMING) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESOURCES) &&
                !record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_PROFILE) &&
                !record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_ZONE_COUNT) &&
                record.ocr_profile ==
                    MADOPILOT_OCR_DIAGNOSTIC_PROFILE_UNSPECIFIED &&
                record.ocr_outcome ==
                    MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_RECOGNIZED &&
                record.ocr_model_instance != 0 &&
                record.frame.sequence == info.value().source.sequence;
        } else if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OCR &&
                   record.result_count == 2) {
            saw_grouped_terminal =
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME) &&
                record.has(
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_SOURCE_ENVELOPE) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_ZONE_COUNT) &&
                record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_COUNTS) &&
                !record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION) &&
                !record.has(
                    MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_REQUESTED_REGION) &&
                !record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_BYTES) &&
                !record.has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_BACKEND_WORK) &&
                record.ocr_zone_count == 3 &&
                record.ocr_unique_candidate_count == 1 &&
                record.ocr_membership_count == 2 &&
                record.ocr_source_envelope.left == 0 &&
                record.ocr_source_envelope.top == 0 &&
                record.ocr_source_envelope.right == 32 &&
                record.ocr_source_envelope.bottom == 12;
        }
    }
    if (!saw_admission || !saw_singular_terminal || !saw_grouped_terminal) {
        std::fprintf(stderr, "OCR diagnostics were incomplete\n");
        return 1;
    }
    std::printf("ocr: sequence=%llu text=%.*s confidence=%.5f\n",
                static_cast<unsigned long long>(info.value().source.sequence),
                static_cast<int>(text.value().size()), text.value().data(),
                region.value().confidence);
    return 0;
}
