#ifndef MADOPILOT_OCR_PRIVATE_FIXTURE_HPP
#define MADOPILOT_OCR_PRIVATE_FIXTURE_HPP

#include "ocr-private-fixture.h"
#include "madopilot/madopilot.hpp"

namespace madopilot::private_fixture {

/// Move-only engine owner constructed only by the feature-gated local fixture.
///
/// This type and its raw constructor adapter are outside installed/public
/// headers. After construction, every operation is inherited unchanged from the
/// ordinary C++ `Engine` wrapper and calls only the negotiated C table.
class OcrEngine : public madopilot::Engine {
public:
    OcrEngine() noexcept = default;
    OcrEngine(const OcrEngine&) = delete;
    OcrEngine& operator=(const OcrEngine&) = delete;
    OcrEngine(OcrEngine&&) noexcept = default;
    OcrEngine& operator=(OcrEngine&&) noexcept = default;

    static madopilot::Result<OcrEngine> create(
        const madopilot::Api& api, const madopilot::Source& source,
        const madopilot::EngineOptions& options,
        const madopilot::Operation& operation) {
        if (!madopilot::detail::has_entry(
                api.table(), api.extent(),
                MADOPILOT_API_SIZE_OCR_RESULT_TEXT_AT)) {
            return madopilot::Result<OcrEngine>::failure(
                madopilot::Error::from_status(MADOPILOT_STATUS_UNSUPPORTED));
        }
        const auto source_c = source.to_c();
        const auto options_c = options.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_engine_t* handle = nullptr;
        ::madopilot_error_t* error = nullptr;
        const madopilot::Status status = ::madopilot_fixture_engine_create(
            &source_c, &options_c, &operation_c, &handle, &error);
        if (!madopilot::is_ok(status)) {
            return madopilot::Result<OcrEngine>::failure(
                madopilot::detail::take_error(api.table(), status, error));
        }
        if (handle == nullptr) {
            return madopilot::Result<OcrEngine>::failure(
                madopilot::Error::from_status(MADOPILOT_STATUS_INTERNAL));
        }

        OcrEngine engine;
        engine.api_ = api.table();
        engine.extent_ = api.extent();
        engine.handle_ = handle;
        return madopilot::Result<OcrEngine>::success(std::move(engine));
    }
};

} // namespace madopilot::private_fixture

#endif /* MADOPILOT_OCR_PRIVATE_FIXTURE_HPP */
